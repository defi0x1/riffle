use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser};
use eyre::WrapErr;

// Precedence, most to least authoritative: CLI flag > environment variable > config file >
// the struct's own compiled default. clap already resolves the first, second and fourth of
// those against each other (a flag beats an env var beats a `default_value`); the trick
// below is to make the file rank strictly *below* an env var without needing to know, field
// by field, whether clap's parse of any given struct got its value from a flag, an env var,
// or its own default. Instead: every OS environment variable the file has an opinion on,
// and that is not already set, gets set from the file before clap ever runs. A real env var
// is left alone, so it still wins; clap then resolves flag-vs-env exactly as it always did.
// Passing `config_path: None` skips all of this and is exactly `T::try_parse_from(args)` --
// no config file, no change from how every binary here behaved before this existed.
pub fn load_config_with_env<T, I, A>(args: I, config_path: Option<&Path>) -> eyre::Result<T>
where
    T: Parser + CommandFactory,
    I: IntoIterator<Item = A>,
    A: Into<std::ffi::OsString> + Clone,
{
    if let Some(path) = config_path {
        let raw = RawConfig::from_config_file(path)?;
        raw.apply_missing_env::<T>()?;
    }

    T::try_parse_from(args).wrap_err_with(|| "Parsing command-line arguments")
}

// Scans for a `--config <path>` / `--config=<path>` flag ahead of the real parse -- the
// config file's path has to be known before `load_config_with_env` can inject anything from
// it, so it cannot itself come from that file. Does not consult the environment: `--config`
// is deliberately CLI-only, so there is exactly one place to look for it.
pub fn config_flag<I, A>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = A>,
    A: AsRef<str>,
{
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let arg = arg.as_ref();
        if let Some(value) = arg.strip_prefix("--config=") {
            return Some(PathBuf::from(value));
        }
        if arg == "--config" {
            return iter.next().map(|v| PathBuf::from(v.as_ref()));
        }
    }
    None
}

/// A parsed config file's raw key/value surface, independent of the struct that will
/// eventually consume it. Kept as its own trait rather than a free function so a second
/// file format could be added later without touching `load_config_with_env`'s call site.
pub trait FromConfigFile: Sized {
    fn from_config_file(path: &Path) -> eyre::Result<Self>;
}

pub struct RawConfig {
    path: PathBuf,
    values: BTreeMap<String, serde_yaml::Value>,
}

impl FromConfigFile for RawConfig {
    fn from_config_file(path: &Path) -> eyre::Result<Self> {
        let text = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("Reading config file {}", path.display()))?;

        let document: serde_yaml::Value = serde_yaml::from_str(&text)
            .wrap_err_with(|| format!("Parsing config file {}", path.display()))?;

        let mapping = match document {
            serde_yaml::Value::Mapping(mapping) => mapping,
            serde_yaml::Value::Null => serde_yaml::Mapping::new(),
            _ => eyre::bail!(
                "Config file {} must be a YAML mapping of key: value settings, not a list or scalar",
                path.display()
            ),
        };

        let mut values = BTreeMap::new();
        for (key, value) in mapping {
            let key = key.as_str().ok_or_else(|| {
                eyre::eyre!("Config file {} has a non-string key", path.display())
            })?;
            values.insert(key.to_string(), value);
        }

        Ok(RawConfig {
            path: path.to_path_buf(),
            values,
        })
    }
}

impl RawConfig {
    // Every arg `T` declares becomes one entry here, keyed by the same snake_case id the
    // YAML file uses (e.g. "database_url"), each paired with whatever environment variable
    // clap resolves that arg from, if any.
    fn apply_missing_env<T: CommandFactory>(&self) -> eyre::Result<()> {
        let command = T::command();
        let known: BTreeMap<String, Option<String>> = command
            .get_arguments()
            .map(|arg| {
                let env = arg.get_env().map(|e| e.to_string_lossy().into_owned());
                (arg.get_id().to_string(), env)
            })
            .collect();

        for (key, value) in &self.values {
            let env_name = match known.get(key) {
                Some(Some(env_name)) => env_name,
                Some(None) => eyre::bail!(
                    "Key '{key}' in config file {} cannot be set from a config file (it has no \
                     environment variable binding)",
                    self.path.display()
                ),
                None => eyre::bail!(
                    "Unknown key '{key}' in config file {} -- not a recognised setting for this binary",
                    self.path.display()
                ),
            };

            if std::env::var_os(env_name).is_some() {
                continue;
            }

            let rendered = render_scalar(value).wrap_err_with(|| {
                format!(
                    "Reading key '{key}' from config file {}",
                    self.path.display()
                )
            })?;

            // Single-threaded startup, before any worker or async task exists to race this.
            unsafe { std::env::set_var(env_name, rendered) };
        }

        Ok(())
    }
}

fn render_scalar(value: &serde_yaml::Value) -> eyre::Result<String> {
    match value {
        serde_yaml::Value::String(s) => Ok(s.clone()),
        serde_yaml::Value::Number(n) => Ok(n.to_string()),
        serde_yaml::Value::Bool(b) => Ok(b.to_string()),
        _ => eyre::bail!("expected a plain string, number or boolean, found a nested value"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    // std::env is process-global; every test here that touches it takes this lock and uses
    // key names unique to itself so parallel test threads cannot interleave.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Parser, Debug, Clone, PartialEq)]
    struct Sample {
        #[arg(long, env = "CONFIG_TEST_FOO")]
        foo: String,

        #[arg(long, env = "CONFIG_TEST_BAR", default_value_t = 7)]
        bar: i64,

        #[arg(long)]
        config: Option<PathBuf>,
    }

    struct EnvGuard(Vec<&'static str>);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for key in &self.0 {
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    fn write_temp_yaml(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "common-config-test-{name}-{}-{}.yaml",
            std::process::id(),
            name
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn test_flag_wins_over_env_and_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard(vec!["CONFIG_TEST_FOO", "CONFIG_TEST_BAR"]);
        unsafe { std::env::set_var("CONFIG_TEST_FOO", "from-env") };

        let path = write_temp_yaml("flag-wins", "foo: from-file\nbar: 1\n");

        let result: Sample =
            load_config_with_env(["prog", "--foo", "from-flag"], Some(path.as_path())).unwrap();

        assert_eq!(result.foo, "from-flag");
        assert_eq!(result.bar, 1); // no flag for bar, env unset, so the file fills it in

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_env_wins_over_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard(vec!["CONFIG_TEST_FOO", "CONFIG_TEST_BAR"]);
        unsafe { std::env::set_var("CONFIG_TEST_FOO", "from-env") };

        let path = write_temp_yaml("env-wins", "foo: from-file\nbar: 2\n");

        let result: Sample = load_config_with_env(["prog"], Some(path.as_path())).unwrap();

        assert_eq!(result.foo, "from-env");
        assert_eq!(result.bar, 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_file_fills_in_when_neither_flag_nor_env_present() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard(vec!["CONFIG_TEST_FOO", "CONFIG_TEST_BAR"]);

        let path = write_temp_yaml("file-only", "foo: from-file\nbar: 3\n");

        let result: Sample = load_config_with_env(["prog"], Some(path.as_path())).unwrap();

        assert_eq!(result.foo, "from-file");
        assert_eq!(result.bar, 3);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_struct_default_still_wins_when_file_omits_the_key() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard(vec!["CONFIG_TEST_FOO", "CONFIG_TEST_BAR"]);

        let path = write_temp_yaml("default-wins", "foo: from-file\n");

        let result: Sample = load_config_with_env(["prog"], Some(path.as_path())).unwrap();

        assert_eq!(result.foo, "from-file");
        assert_eq!(result.bar, 7); // clap's own default_value_t, file said nothing about it

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_absent_config_path_behaves_exactly_like_plain_parse() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard(vec!["CONFIG_TEST_FOO", "CONFIG_TEST_BAR"]);
        unsafe { std::env::set_var("CONFIG_TEST_FOO", "from-env") };

        let result: Sample = load_config_with_env(["prog"], None).unwrap();

        assert_eq!(result.foo, "from-env");
        assert_eq!(result.bar, 7);
    }

    #[test]
    fn test_unknown_key_is_rejected_by_name() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard(vec!["CONFIG_TEST_FOO", "CONFIG_TEST_BAR"]);

        let path = write_temp_yaml(
            "unknown-key",
            "foo: from-file\nbar: 4\ntotally_bogus_setting: 1\n",
        );

        let err = load_config_with_env::<Sample, _, _>(["prog"], Some(path.as_path())).unwrap_err();

        let message = format!("{err:#}");
        assert!(
            message.contains("totally_bogus_setting"),
            "error should name the offending key, got: {message}"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_missing_file_errors_with_path() {
        let path = PathBuf::from("/nonexistent/path/for/common-config-test.yaml");
        let err = load_config_with_env::<Sample, _, _>(["prog"], Some(path.as_path())).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains(&path.display().to_string()));
    }
}
