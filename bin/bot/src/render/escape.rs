// Telegram's MarkdownV2 rejects the whole message if any of these characters appears
// unescaped outside a code span. Anything that embeds text we did not author ourselves --
// a pool address, a clap error, a rationale note -- has to go through this first, or a
// stray `.` or `-` in that text breaks the parse for the entire message.
const SPECIAL: &[char] = &[
    '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!', '\\',
];

pub fn escape_markdown_v2(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if SPECIAL.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// Inside a MarkdownV2 code span (`` `...` ``) only the backtick and the backslash itself
// need escaping -- everything else, periods and dashes included, is literal. Used for
// values placed in monospace (addresses, numbers, timestamps) so they never need the full
// escape set.
pub fn escape_code_span(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if c == '`' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escapes_every_special_character() {
        for c in SPECIAL {
            let escaped = escape_markdown_v2(&c.to_string());
            assert_eq!(escaped, format!("\\{c}"), "char {c:?} was not escaped");
        }
    }

    #[test]
    fn test_leaves_plain_text_untouched() {
        assert_eq!(escape_markdown_v2("hello world 123"), "hello world 123");
    }

    #[test]
    fn test_escapes_a_clap_error_style_string() {
        let input = "error: unexpected argument '--off' found (pool 7xKX...tv1)";
        let escaped = escape_markdown_v2(input);
        assert!(!escaped.contains("--off'"));
        assert!(escaped.contains("\\-\\-off"));
        assert!(escaped.contains("\\("));
        assert!(escaped.contains("\\)"));
    }

    #[test]
    fn test_code_span_escapes_only_backtick_and_backslash() {
        assert_eq!(escape_code_span("a.b-c(d)"), "a.b-c(d)");
        assert_eq!(escape_code_span("a`b"), "a\\`b");
        assert_eq!(escape_code_span("a\\b"), "a\\\\b");
    }

    #[test]
    fn test_pool_address_with_underscores_and_digits_round_trips_visually() {
        // Base58 addresses never contain markdown-special characters, so escaping is a
        // no-op for the realistic case -- checked here so a future change to SPECIAL
        // cannot silently start mangling addresses.
        let addr = "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU";
        assert_eq!(escape_markdown_v2(addr), addr);
    }
}
