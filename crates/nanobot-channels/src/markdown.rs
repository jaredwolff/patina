//! Markdown-to-Telegram-HTML converter.
//!
//! Telegram supports a limited HTML subset: `<b>`, `<i>`, `<s>`, `<code>`,
//! `<pre>`, `<a href="">`. This module converts standard markdown into that
//! subset, matching the Python `_markdown_to_telegram_html()` implementation.

use regex::Regex;

/// Replace `_text_` with `<i>text</i>`, but only when the underscores are
/// not surrounded by word characters (to avoid matching `some_var_name`).
fn replace_italic(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '_' {
            // Check that preceding char is not alphanumeric
            let prev_is_word = i > 0 && chars[i - 1].is_alphanumeric();
            if !prev_is_word {
                // Find the closing underscore
                let remainder: String = chars[i + 1..].iter().collect();
                if let Some(end) = remainder.find('_') {
                    let end_pos = i + 1 + end;
                    // Check that the char after closing underscore is not alphanumeric
                    let next_is_word = end_pos + 1 < chars.len() && chars[end_pos + 1].is_alphanumeric();
                    if !next_is_word && end > 0 {
                        result.push_str("<i>");
                        let italic_text: String = chars[i + 1..end_pos].iter().collect();
                        result.push_str(&italic_text);
                        result.push_str("</i>");
                        i = end_pos + 1;
                        continue;
                    }
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Convert markdown text to Telegram-safe HTML.
pub fn markdown_to_telegram_html(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let mut text = text.to_string();

    // 1. Extract and protect code blocks (``` ... ```)
    let mut code_blocks: Vec<String> = Vec::new();
    let re_code_block = Regex::new(r"```[\w]*\n?([\s\S]*?)```").unwrap();
    text = re_code_block
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = code_blocks.len();
            code_blocks.push(caps[1].to_string());
            format!("\x00CB{idx}\x00")
        })
        .into_owned();

    // 2. Extract and protect inline code (` ... `)
    let mut inline_codes: Vec<String> = Vec::new();
    let re_inline = Regex::new(r"`([^`]+)`").unwrap();
    text = re_inline
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = inline_codes.len();
            inline_codes.push(caps[1].to_string());
            format!("\x00IC{idx}\x00")
        })
        .into_owned();

    // 3. Convert markdown tables to monospaced pre blocks
    let mut table_blocks: Vec<String> = Vec::new();
    let re_table = Regex::new(r"(?m)(?:^\|.+\|$\n?)+").unwrap();
    text = re_table
        .replace_all(&text, |caps: &regex::Captures| {
            let table_text = caps[0].trim();
            let lines: Vec<&str> = table_text.lines().collect();
            let mut rows: Vec<Vec<String>> = Vec::new();

            for line in &lines {
                let stripped = line.trim().trim_matches('|');
                // Skip separator rows (e.g. |---|---|)
                if !stripped.is_empty() && stripped.chars().all(|c| "-: |".contains(c)) {
                    continue;
                }
                let cells: Vec<String> =
                    stripped.split('|').map(|c| c.trim().to_string()).collect();
                if !cells.is_empty() {
                    rows.push(cells);
                }
            }

            if rows.is_empty() {
                return caps[0].to_string();
            }

            // Compute column widths
            let n_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
            for row in &mut rows {
                while row.len() < n_cols {
                    row.push(String::new());
                }
            }
            let widths: Vec<usize> = (0..n_cols)
                .map(|c| rows.iter().map(|r| r[c].len()).max().unwrap_or(0))
                .collect();

            // Format as aligned plain text
            let mut fmt_lines: Vec<String> = Vec::new();
            for (i, row) in rows.iter().enumerate() {
                let line: String = row
                    .iter()
                    .zip(&widths)
                    .map(|(cell, &w)| format!("{:<width$}", cell, width = w))
                    .collect::<Vec<_>>()
                    .join("  ");
                fmt_lines.push(line);
                if i == 0 {
                    let sep: String = widths
                        .iter()
                        .map(|&w| "-".repeat(w))
                        .collect::<Vec<_>>()
                        .join("  ");
                    fmt_lines.push(sep);
                }
            }

            let block = fmt_lines.join("\n");
            let idx = table_blocks.len();
            table_blocks.push(block);
            format!("\x00TB{idx}\x00")
        })
        .into_owned();

    // 4. Strip headers (# Title -> Title)
    let re_headers = Regex::new(r"(?m)^#{1,6}\s+(.+)$").unwrap();
    text = re_headers.replace_all(&text, "$1").into_owned();

    // 5. Strip blockquotes (> text -> text)
    let re_blockquote = Regex::new(r"(?m)^>\s*(.*)$").unwrap();
    text = re_blockquote.replace_all(&text, "$1").into_owned();

    // 6. Escape HTML special characters
    text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    // 7. Links [text](url) - must be before bold/italic to handle nested cases
    let re_links = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    text = re_links
        .replace_all(&text, r#"<a href="$2">$1</a>"#)
        .into_owned();

    // 8. Bold **text** or __text__
    let re_bold_star = Regex::new(r"\*\*(.+?)\*\*").unwrap();
    text = re_bold_star.replace_all(&text, "<b>$1</b>").into_owned();
    let re_bold_under = Regex::new(r"__(.+?)__").unwrap();
    text = re_bold_under.replace_all(&text, "<b>$1</b>").into_owned();

    // 9. Italic _text_ (avoid matching inside words like some_var_name)
    //    Rust regex doesn't support lookbehind/lookahead, so we use a manual approach.
    text = replace_italic(&text);

    // 10. Strikethrough ~~text~~
    let re_strike = Regex::new(r"~~(.+?)~~").unwrap();
    text = re_strike.replace_all(&text, "<s>$1</s>").into_owned();

    // 11. Bullet lists - item -> bullet item
    let re_bullet = Regex::new(r"(?m)^[-*]\s+").unwrap();
    text = re_bullet.replace_all(&text, "\u{2022} ").into_owned();

    // 12. Restore inline code with HTML tags
    for (i, code) in inline_codes.iter().enumerate() {
        let escaped = code
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        text = text.replace(
            &format!("\x00IC{i}\x00"),
            &format!("<code>{escaped}</code>"),
        );
    }

    // 13. Restore code blocks with HTML tags
    for (i, code) in code_blocks.iter().enumerate() {
        let escaped = code
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        text = text.replace(
            &format!("\x00CB{i}\x00"),
            &format!("<pre><code>{escaped}</code></pre>"),
        );
    }

    // 14. Restore table blocks as monospaced pre blocks
    for (i, block) in table_blocks.iter().enumerate() {
        let escaped = block
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        text = text.replace(&format!("\x00TB{i}\x00"), &format!("<pre>{escaped}</pre>"));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        assert_eq!(markdown_to_telegram_html(""), "");
    }

    #[test]
    fn test_plain_text() {
        assert_eq!(markdown_to_telegram_html("hello world"), "hello world");
    }

    #[test]
    fn test_bold() {
        assert_eq!(
            markdown_to_telegram_html("this is **bold** text"),
            "this is <b>bold</b> text"
        );
    }

    #[test]
    fn test_italic() {
        assert_eq!(
            markdown_to_telegram_html("this is _italic_ text"),
            "this is <i>italic</i> text"
        );
    }

    #[test]
    fn test_italic_not_in_words() {
        // Should NOT match underscores inside words
        assert_eq!(markdown_to_telegram_html("some_var_name"), "some_var_name");
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(
            markdown_to_telegram_html("this is ~~deleted~~ text"),
            "this is <s>deleted</s> text"
        );
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(
            markdown_to_telegram_html("use `println!` here"),
            "use <code>println!</code> here"
        );
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<pre><code>"));
        assert!(output.contains("fn main()"));
        assert!(output.contains("</code></pre>"));
    }

    #[test]
    fn test_html_escaping() {
        assert_eq!(
            markdown_to_telegram_html("a < b & c > d"),
            "a &lt; b &amp; c &gt; d"
        );
    }

    #[test]
    fn test_html_in_code_block() {
        let input = "```\n<div>hello</div>\n```";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("&lt;div&gt;"));
    }

    #[test]
    fn test_link() {
        assert_eq!(
            markdown_to_telegram_html("[click here](https://example.com)"),
            r#"<a href="https://example.com">click here</a>"#
        );
    }

    #[test]
    fn test_header_stripped() {
        assert_eq!(markdown_to_telegram_html("# Hello"), "Hello");
        assert_eq!(markdown_to_telegram_html("### Sub heading"), "Sub heading");
    }

    #[test]
    fn test_blockquote_stripped() {
        assert_eq!(markdown_to_telegram_html("> quoted text"), "quoted text");
    }

    #[test]
    fn test_bullet_list() {
        let input = "- item one\n- item two\n* item three";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("\u{2022} item one"));
        assert!(output.contains("\u{2022} item two"));
        assert!(output.contains("\u{2022} item three"));
    }

    #[test]
    fn test_table() {
        let input = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<pre>"));
        assert!(output.contains("Alice"));
        assert!(output.contains("Bob"));
        assert!(output.contains("</pre>"));
    }

    #[test]
    fn test_combined() {
        let input = "# Title\n\nSome **bold** and _italic_ with `code`.\n\n```\nlet x = 1;\n```";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("Title"));
        assert!(output.contains("<b>bold</b>"));
        assert!(output.contains("<i>italic</i>"));
        assert!(output.contains("<code>code</code>"));
        assert!(output.contains("<pre><code>"));
    }

    #[test]
    fn test_utf8_smart_quotes() {
        // Test that UTF-8 characters (smart quotes, em-dashes, etc.) are preserved
        let input = "I'm a test—with smart quotes and em-dashes";
        let output = markdown_to_telegram_html(input);
        assert_eq!(output, "I'm a test—with smart quotes and em-dashes");
    }

    #[test]
    fn test_utf8_with_italic() {
        // Test UTF-8 characters combined with markdown formatting
        let input = "I'm _really_ excited—**this** works!";
        let output = markdown_to_telegram_html(input);
        assert_eq!(output, "I'm <i>really</i> excited—<b>this</b> works!");
    }

    #[test]
    fn test_utf8_emoji() {
        // Test emoji preservation
        let input = "Hello 👋 world 🌍";
        let output = markdown_to_telegram_html(input);
        assert_eq!(output, "Hello 👋 world 🌍");
    }
}
