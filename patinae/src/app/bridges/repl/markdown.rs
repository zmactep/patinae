//! Render Markdown blocks using Slint's inline text styling.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

pub(super) struct Block {
    pub text: slint::StyledText,
    pub plain_text: String,
    pub heading_level: i32,
    pub code_block: bool,
}

/// Keep unsupported block syntax from disabling styling in adjacent paragraphs.
pub(super) fn render(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut heading_level = 0;
    let mut code_block = false;
    let mut plain = String::new();
    for (event, range) in Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS,
    )
    .into_offset_iter()
    {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    start = range.start;
                    heading_level = match tag {
                        Tag::Heading { level, .. } => level as i32,
                        _ => 0,
                    };
                    code_block = matches!(tag, Tag::CodeBlock(_) | Tag::Table(_));
                    plain.clear();
                }
                depth += 1;
            }
            Event::End(tag) => {
                match tag {
                    TagEnd::TableCell => plain.push_str("   "),
                    TagEnd::Paragraph | TagEnd::Item | TagEnd::TableHead | TagEnd::TableRow
                        if !plain.ends_with('\n') =>
                    {
                        plain.push('\n');
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let text = if heading_level > 0 || code_block {
                        slint::StyledText::from_plain_text(plain.trim_end_matches('\n'))
                    } else {
                        slint::StyledText::from_markdown(&source[start..range.end]).unwrap_or_else(
                            |_| slint::StyledText::from_plain_text(plain.trim_end()),
                        )
                    };
                    blocks.push(Block {
                        text,
                        plain_text: plain.trim_end_matches('\n').into(),
                        heading_level,
                        code_block,
                    });
                }
            }
            Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text) => {
                plain.push_str(&text);
                if depth == 0 {
                    blocks.push(Block {
                        text: slint::StyledText::from_plain_text(&text),
                        plain_text: text.to_string(),
                        heading_level: 0,
                        code_block: false,
                    });
                }
            }
            Event::SoftBreak | Event::HardBreak => plain.push('\n'),
            Event::TaskListMarker(checked) => plain.push_str(if checked { "☑ " } else { "☐ " }),
            Event::Rule => blocks.push(Block {
                text: slint::StyledText::from_plain_text("────────────────"),
                plain_text: "────────────────".into(),
                heading_level: 0,
                code_block: false,
            }),
            Event::FootnoteReference(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                plain.push_str(&text);
            }
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_and_code_do_not_disable_inline_formatting() {
        let blocks = render("# Result\n\n**Bold** and *italic* with `code`.\n\n- first\n- second\n\n```python\n# literal\nprint('**text**')\n```\n");
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].heading_level, 1);
        assert_eq!(blocks[0].text, slint::StyledText::from_plain_text("Result"));
        assert_eq!(
            blocks[1].text,
            slint::StyledText::from_markdown("**Bold** and *italic* with `code`.").unwrap()
        );
        assert_eq!(
            blocks[2].text,
            slint::StyledText::from_markdown("- first\n- second").unwrap()
        );
        assert!(blocks[3].code_block);
        assert_eq!(
            blocks[3].text,
            slint::StyledText::from_plain_text("# literal\nprint('**text**')")
        );
    }

    #[test]
    fn unsupported_markup_retains_content_and_following_styles() {
        let blocks = render("> quoted text\n\n| A | B |\n|---|---|\n| one | two |\n\n**After**");
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks[0].text,
            slint::StyledText::from_plain_text("quoted text")
        );
        assert!(blocks[1].code_block);
        assert_eq!(
            blocks[2].text,
            slint::StyledText::from_markdown("**After**").unwrap()
        );
    }
}
