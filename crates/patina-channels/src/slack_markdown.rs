//! Markdown-to-Slack-mrkdwn converter.
//!
//! Slack uses its own "mrkdwn" format which differs from standard markdown:
//! - Bold: `*text*` (not `**text**`)
//! - Italic: `_text_` (same)
//! - Strikethrough: `~text~` (not `~~text~~`)
//! - Links: `<url|text>` (not `[text](url)`)
//! - Code: `` `code` `` and ` ```code``` ` (same)
//! - No headers — converted to bold
//! - Special chars `<`, `>`, `&` must be escaped

use regex::Regex;

/// Convert markdown text to Slack mrkdwn format.
pub fn markdown_to_slack_mrkdwn(text: &str) -> String {
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

    // 3. Convert markdown tables to monospaced code blocks
    let mut table_blocks: Vec<String> = Vec::new();
    let re_table = Regex::new(r"(?m)(?:^\|.+\|$\n?)+").unwrap();
    text = re_table
        .replace_all(&text, |caps: &regex::Captures| {
            let table_text = caps[0].trim();
            let lines: Vec<&str> = table_text.lines().collect();
            let mut rows: Vec<Vec<String>> = Vec::new();

            for line in &lines {
                let stripped = line.trim().trim_matches('|');
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

            let n_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
            for row in &mut rows {
                while row.len() < n_cols {
                    row.push(String::new());
                }
            }
            let widths: Vec<usize> = (0..n_cols)
                .map(|c| rows.iter().map(|r| r[c].len()).max().unwrap_or(0))
                .collect();

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

    // 4. Escape Slack special characters: &, <, >
    // Must happen before we insert Slack markup that uses < and >
    text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    // 5. Links [text](url) → <url|text>
    let re_links = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    text = re_links.replace_all(&text, "<$2|$1>").into_owned();

    // 6. Bold **text** or __text__ → *text*
    let re_bold_star = Regex::new(r"\*\*(.+?)\*\*").unwrap();
    text = re_bold_star.replace_all(&text, "*$1*").into_owned();
    let re_bold_under = Regex::new(r"__(.+?)__").unwrap();
    text = re_bold_under.replace_all(&text, "*$1*").into_owned();

    // 7. Italic _text_ stays as _text_ in Slack mrkdwn (same syntax).
    //    No conversion needed — but we still need to avoid matching inside words.
    //    Slack handles this natively, so we leave it as-is.

    // 8. Strikethrough ~~text~~ → ~text~
    let re_strike = Regex::new(r"~~(.+?)~~").unwrap();
    text = re_strike.replace_all(&text, "~$1~").into_owned();

    // 9. Headers # Title → *Title* (bold, Slack has no header syntax)
    let re_headers = Regex::new(r"(?m)^#{1,6}\s+(.+)$").unwrap();
    text = re_headers.replace_all(&text, "*$1*").into_owned();

    // 10. Blockquotes > text → > text (same syntax, no change needed)

    // 11. Bullet lists - item → • item
    let re_bullet = Regex::new(r"(?m)^[-*]\s+").unwrap();
    text = re_bullet.replace_all(&text, "\u{2022} ").into_owned();

    // 12. Restore inline code (no escaping needed — Slack handles code literally)
    for (i, code) in inline_codes.iter().enumerate() {
        text = text.replace(&format!("\x00IC{i}\x00"), &format!("`{code}`"));
    }

    // 13. Restore code blocks
    for (i, code) in code_blocks.iter().enumerate() {
        text = text.replace(&format!("\x00CB{i}\x00"), &format!("```{code}```"));
    }

    // 14. Restore table blocks as code blocks
    for (i, block) in table_blocks.iter().enumerate() {
        text = text.replace(&format!("\x00TB{i}\x00"), &format!("```{block}```"));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        assert_eq!(markdown_to_slack_mrkdwn(""), "");
    }

    #[test]
    fn test_plain_text() {
        assert_eq!(markdown_to_slack_mrkdwn("hello world"), "hello world");
    }

    #[test]
    fn test_bold() {
        assert_eq!(
            markdown_to_slack_mrkdwn("this is **bold** text"),
            "this is *bold* text"
        );
    }

    #[test]
    fn test_italic() {
        assert_eq!(
            markdown_to_slack_mrkdwn("this is _italic_ text"),
            "this is _italic_ text"
        );
    }

    #[test]
    fn test_italic_not_in_words() {
        assert_eq!(markdown_to_slack_mrkdwn("some_var_name"), "some_var_name");
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(
            markdown_to_slack_mrkdwn("this is ~~deleted~~ text"),
            "this is ~deleted~ text"
        );
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(
            markdown_to_slack_mrkdwn("use `println!` here"),
            "use `println!` here"
        );
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
        let output = markdown_to_slack_mrkdwn(input);
        assert!(output.starts_with("```"));
        assert!(output.ends_with("```"));
        assert!(output.contains("fn main()"));
    }

    #[test]
    fn test_special_char_escaping() {
        assert_eq!(
            markdown_to_slack_mrkdwn("a < b & c > d"),
            "a &lt; b &amp; c &gt; d"
        );
    }

    #[test]
    fn test_link() {
        assert_eq!(
            markdown_to_slack_mrkdwn("[click here](https://example.com)"),
            "<https://example.com|click here>"
        );
    }

    #[test]
    fn test_header_to_bold() {
        assert_eq!(markdown_to_slack_mrkdwn("# Hello"), "*Hello*");
        assert_eq!(markdown_to_slack_mrkdwn("### Sub heading"), "*Sub heading*");
    }

    #[test]
    fn test_blockquote() {
        assert_eq!(
            markdown_to_slack_mrkdwn("> quoted text"),
            "&gt; quoted text"
        );
    }

    #[test]
    fn test_bullet_list() {
        let input = "- item one\n- item two";
        let output = markdown_to_slack_mrkdwn(input);
        assert!(output.contains("\u{2022} item one"));
        assert!(output.contains("\u{2022} item two"));
    }

    #[test]
    fn test_table() {
        let input = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |";
        let output = markdown_to_slack_mrkdwn(input);
        assert!(output.contains("```"));
        assert!(output.contains("Alice"));
        assert!(output.contains("Bob"));
    }

    #[test]
    fn test_combined() {
        let input = "# Title\n\nSome **bold** and _italic_ with `code`.";
        let output = markdown_to_slack_mrkdwn(input);
        assert!(output.contains("*Title*"));
        assert!(output.contains("*bold*"));
        assert!(output.contains("_italic_"));
        assert!(output.contains("`code`"));
    }

    #[test]
    fn test_special_chars_in_code_not_escaped() {
        let input = "`a < b & c > d`";
        let output = markdown_to_slack_mrkdwn(input);
        assert_eq!(output, "`a < b & c > d`");
    }

    #[test]
    fn test_utf8_preserved() {
        assert_eq!(
            markdown_to_slack_mrkdwn("Hello 👋 world 🌍"),
            "Hello 👋 world 🌍"
        );
    }
}
