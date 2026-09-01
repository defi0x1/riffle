// Telegram caps a single message at 4096 characters. A `/why` dump with a long rationale
// trail routinely exceeds that, so it is paginated rather than cut short -- every line of
// rationale is a reason someone might need to see, not a nice-to-have to drop.
pub const MESSAGE_LIMIT: usize = 4096;

// Room left for the "(continued i/n)" footer, sized generously (three-digit page counts)
// so the second packing pass never has to trim a page to make the footer fit.
const MARKER_RESERVE: usize = 48;

// Splits on line boundaries only, so a backslash-escaped MarkdownV2 character (`\.`, `\-`,
// ...) is never separated from the backslash that escapes it -- doing that would leave a
// dangling backslash and break parsing on Telegram's side for whichever page it landed on.
// Never drops content: a message that does not fit is split into more pages, not shortened.
pub fn paginate(text: &str, limit: usize) -> Vec<String> {
    let pages = pack_lines(text, limit);
    if pages.len() <= 1 {
        return pages;
    }

    let reserved_limit = limit.saturating_sub(MARKER_RESERVE).max(1);
    let mut pages = pack_lines(text, reserved_limit);
    let total = pages.len();
    for (i, page) in pages.iter_mut().enumerate() {
        page.push_str(&format!("\n\n_(continued {}/{total})_", i + 1));
    }
    pages
}

fn pack_lines(text: &str, limit: usize) -> Vec<String> {
    let mut pages = Vec::new();
    let mut current = String::new();

    for line in text.split('\n') {
        if line.len() > limit {
            if !current.is_empty() {
                pages.push(std::mem::take(&mut current));
            }
            pages.extend(split_long_line(line, limit));
            continue;
        }

        let extra = if current.is_empty() {
            line.len()
        } else {
            line.len() + 1
        };
        if current.len() + extra > limit {
            pages.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() || pages.is_empty() {
        pages.push(current);
    }

    pages
}

// Only reached when a single line is longer than the whole page budget -- rare, but a
// pathological rationale note could do it. Cuts on a char boundary, and never right after a
// trailing backslash, so the escape pair it opens stays intact in the next chunk.
fn split_long_line(line: &str, limit: usize) -> Vec<String> {
    let mut rest = line;
    let mut out = Vec::new();

    while rest.len() > limit {
        let mut idx = limit.min(rest.len());
        while idx > 0 && !rest.is_char_boundary(idx) {
            idx -= 1;
        }
        while idx > 0 && rest.as_bytes()[idx - 1] == b'\\' {
            idx -= 1;
        }
        if idx == 0 {
            // Nothing but backslashes up to the limit -- cut at the raw limit rather than
            // spin forever; this is not a case real rationale text produces.
            idx = limit.min(rest.len());
        }
        out.push(rest[..idx].to_string());
        rest = &rest[idx..];
    }

    if !rest.is_empty() {
        out.push(rest.to_string());
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_message_is_a_single_page() {
        let pages = paginate("short message", MESSAGE_LIMIT);
        assert_eq!(pages, vec!["short message".to_string()]);
    }

    #[test]
    fn test_long_message_splits_into_multiple_pages() {
        let line = "x".repeat(100);
        let text = std::iter::repeat_n(line, 60).collect::<Vec<_>>().join("\n");
        let pages = paginate(&text, MESSAGE_LIMIT);
        assert!(pages.len() > 1);
        for page in &pages {
            assert!(page.len() <= MESSAGE_LIMIT);
        }
    }

    #[test]
    fn test_pages_carry_a_continuation_marker_except_none_when_single() {
        let single = paginate("short", MESSAGE_LIMIT);
        assert!(!single[0].contains("continued"));

        let line = "x".repeat(100);
        let text = std::iter::repeat_n(line, 60).collect::<Vec<_>>().join("\n");
        let pages = paginate(&text, MESSAGE_LIMIT);
        let total = pages.len();
        for (i, page) in pages.iter().enumerate() {
            assert!(page.contains(&format!("continued {}/{total}", i + 1)));
        }
    }

    #[test]
    fn test_split_never_breaks_an_escaped_character_pair() {
        // A run of escaped periods, as escape_markdown_v2 would produce from "1.2.3...".
        let escaped_run = "\\.".repeat(4000);
        let pages = paginate(&escaped_run, MESSAGE_LIMIT);
        assert!(pages.len() > 1);
        for page in &pages {
            let body = page.split("\n\n_(continued").next().unwrap_or(page);
            // An odd number of trailing backslashes means the last one is dangling --
            // it opened an escape pair that got cut off from its character.
            let trailing_backslashes = body.chars().rev().take_while(|&c| c == '\\').count();
            assert_eq!(
                trailing_backslashes % 2,
                0,
                "page ends mid-escape: {body:?}"
            );
        }
    }

    #[test]
    fn test_pagination_preserves_every_line_of_content() {
        let lines: Vec<String> = (0..500)
            .map(|i| format!("row {i}: some rationale text"))
            .collect();
        let original = lines.join("\n");

        let pages = paginate(&original, MESSAGE_LIMIT);
        assert!(pages.len() > 1);

        let mut reconstructed = String::new();
        for (i, page) in pages.iter().enumerate() {
            let body = page.split("\n\n_(continued").next().unwrap_or(page);
            if i > 0 {
                reconstructed.push('\n');
            }
            reconstructed.push_str(body);
        }

        assert_eq!(reconstructed, original);
    }
}
