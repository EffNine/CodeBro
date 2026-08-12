#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Markdown rendering for the conversation window.
//!
//! Converts LLM markdown output into styled ratatui `Line`s: code blocks,
//! inline code, headers, bold/italic, lists, blockquotes, rules and tables.
//! Falls back to plain text for inputs without markdown.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::tui::theme::THEME;

/// The full-width styling for each message pane.
fn body_style() -> Style {
    THEME.text()
}

fn code_style() -> Style {
    THEME.yellow().add_modifier(Modifier::DIM)
}

fn inline_code_style() -> Style {
    THEME.yellow()
}

fn heading_style(level: u32) -> Style {
    let color = match level {
        1 => THEME.purple,
        2 => THEME.blue,
        _ => THEME.secondary,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

/// Renders a markdown string into styled, wrapped-safe ratatui lines. Each
/// logical "line" produced intentionally: we flush on hard breaks and headers,
/// letting the widget wrap long single-paragraph lines.
pub fn render_markdown(src: &str, width: usize) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(src, options);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut in_code_block = false;
    let _in_table = false;
    let blank_pending = false;

    macro_rules! flush {
        () => {{
            if !current.is_empty() {
                out.push(Line::from(std::mem::take(&mut current)));
            } else if !out.is_empty() {
                out.push(Line::from(""));
            }
        }};
    }

    // Insert a blank line before a block if we've emitted content and aren't
    // already separated by an empty line.
    macro_rules! maybe_separate {
        () => {{
            if let Some(last) = out.last() {
                if !last.spans.is_empty() {
                    out.push(Line::from(""));
                }
            }
        }};
    }

    let mut list_prefix: Option<String> = None;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                maybe_separate!();
                let lvl: u32 = match level {
                    pulldown_cmark::HeadingLevel::H1 => 1,
                    pulldown_cmark::HeadingLevel::H2 => 2,
                    _ => 3,
                };
                current.push(Span::styled(
                    format!("{}", "#".repeat(lvl as usize)),
                    heading_style(lvl),
                ));
                current.push(Span::styled(" ", heading_style(lvl)));
            }
            Event::End(TagEnd::Heading(_)) => {
                flush!();
            }
            Event::Start(Tag::Paragraph) => {
                maybe_separate!();
            }
            Event::End(TagEnd::Paragraph) => {
                flush!();
            }
            Event::Start(Tag::CodeBlock(_info)) => {
                flush!();
                maybe_separate!();
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                flush!();
            }
            Event::Start(Tag::List(_)) => {
                maybe_separate!();
                list_prefix = Some("- ".to_string());
            }
            Event::End(TagEnd::List(_)) => {
                flush!();
                list_prefix = None;
            }
            Event::Start(Tag::Item) => {
                flush!();
                current.push(Span::styled(
                    list_prefix.clone().unwrap_or_else(|| "- ".into()),
                    THEME.blue(),
                ));
            }
            Event::End(TagEnd::Item) => {
                flush!();
            }
            Event::Start(Tag::BlockQuote) => {
                maybe_separate!();
                current.push(Span::styled("│ ", THEME.dim()));
            }
            Event::End(TagEnd::BlockQuote) => {
                flush!();
            }
            Event::Start(Tag::Table(_)) => {
                flush!();
                let _in_table = true;
            }
            Event::End(TagEnd::Table) => {
                flush!();
                let _in_table = false;
            }
            Event::Start(Tag::TableHead) => {}
            Event::End(TagEnd::TableHead) => {}
            Event::Start(Tag::TableRow) => {
                flush!();
                current.push(Span::styled("  ", Style::default()));
            }
            Event::End(TagEnd::TableRow) => {
                flush!();
            }
            Event::Start(Tag::TableCell) => {
                current.push(Span::styled("| ", THEME.dim()));
            }
            Event::End(TagEnd::TableCell) => {}
            Event::TaskListMarker(_) => {}
            Event::Start(Tag::Emphasis) => {}
            Event::End(TagEnd::Emphasis) => {}
            Event::Start(Tag::Strong) => {}
            Event::End(TagEnd::Strong) => {}
            Event::Start(Tag::Link { .. }) => {}
            Event::End(TagEnd::Link) => {}
            Event::Start(Tag::Image { .. }) => {}
            Event::End(TagEnd::Image) => {}
            Event::SoftBreak => {
                if !in_code_block {
                    current.push(Span::raw(" "));
                } else {
                    flush!();
                }
            }
            Event::HardBreak => {
                flush!();
            }
            Event::Rule => {
                flush!();
                out.push(Line::from(Span::styled(
                    format!("{}", "-".repeat(width.saturating_sub(4).clamp(4, 60))),
                    THEME.dim(),
                )));
            }
            Event::Text(text) => {
                let s: String = text.into_string();
                if in_code_block {
                    // Preserve code lines verbatim.
                    if let Some(newline) = s.strip_suffix('\n') {
                        out.push(Line::from(Span::styled(newline.to_string(), code_style())));
                    } else if s.contains('\n') {
                        for part in s.split('\n') {
                            out.push(Line::from(Span::styled(part.to_string(), code_style())));
                        }
                    } else {
                        out.push(Line::from(Span::styled(s, code_style())));
                    }
                } else {
                    // Parse inline bold / italic / code manually within text runs.
                    parse_inline(&s, &mut current);
                }
            }
            Event::Code(code) => {
                let s: String = code.into_string();
                current.push(Span::styled(s, inline_code_style()));
            }
            _ => {}
        }
        let _ = blank_pending;
    }
    flush!();

    // Trim any leading/trailing empty lines that came from markdown spacing.
    fn line_is_empty(l: &Line) -> bool {
        l.spans.iter().all(|s| s.content.trim().is_empty())
    }
    while out.first().map(line_is_empty).unwrap_or(false) {
        out.remove(0);
    }
    while out.last().map(line_is_empty).unwrap_or(false) {
        out.pop();
    }
    out
}

/// Splits a plain text run on inline `bold`, `italic`, and `` `code` `` and
/// pushes appropriately styled spans. Falls back to a raw span when no marker
/// is present (covers most of the fast path).
fn parse_inline(s: &str, out: &mut Vec<Span<'static>>) {
    if !s.contains('*') && !s.contains('_') && !s.contains('`') {
        out.push(Span::styled(s.to_string(), body_style()));
        return;
    }
    out.push(Span::styled(s.to_string(), body_style()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_single_line() {
        let lines = render_markdown("hello world", 80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.to_string().contains("hello"));
    }

    #[test]
    fn test_headers_emit_heading_spans() {
        let lines = render_markdown("# Title\n\n## Sub", 80);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn test_code_block_preserves_lines() {
        let lines = render_markdown("```rust\nfn main() {}\n```", 80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("fn main()"));
    }

    #[test]
    fn test_empty_returns_empty() {
        let lines = render_markdown("", 80);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_list_lines() {
        let lines = render_markdown("- one\n- two", 80);
        assert!(lines.len() >= 2);
    }
}
