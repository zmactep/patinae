use std::rc::Rc;

use slint::{ModelRc, VecModel};

use patinae_framework::plugin_ui::PanelTextHighlight as CoreHighlight;

use crate::{PluginHighlightLine, PluginHighlightRun};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HighlightRun {
    pub text: String,
    pub style: String,
    pub start_col: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HighlightLine {
    pub runs: Vec<HighlightRun>,
}

pub(crate) fn empty_highlight_lines_model() -> ModelRc<PluginHighlightLine> {
    ModelRc::from(Rc::new(VecModel::default()))
}

pub(crate) fn highlight_lines_model(
    source: &str,
    highlights: &[CoreHighlight],
) -> ModelRc<PluginHighlightLine> {
    let rows: Vec<PluginHighlightLine> = build_highlight_lines(source, highlights)
        .into_iter()
        .map(|line| PluginHighlightLine {
            runs: ModelRc::from(Rc::new(VecModel::from(
                line.runs
                    .into_iter()
                    .map(|run| PluginHighlightRun {
                        text: run.text.into(),
                        style: run.style.into(),
                        start_col: run.start_col,
                    })
                    .collect::<Vec<_>>(),
            ))),
        })
        .collect();

    if rows.is_empty() {
        empty_highlight_lines_model()
    } else {
        ModelRc::from(Rc::new(VecModel::from(rows)))
    }
}

pub(crate) fn build_highlight_lines(
    source: &str,
    highlights: &[CoreHighlight],
) -> Vec<HighlightLine> {
    if source.is_empty() || highlights.is_empty() {
        return Vec::new();
    }

    let mut spans: Vec<_> = highlights
        .iter()
        .filter(|highlight| {
            highlight.start < highlight.end
                && highlight.end <= source.len()
                && source.is_char_boundary(highlight.start)
                && source.is_char_boundary(highlight.end)
        })
        .collect();
    spans.sort_by_key(|highlight| (highlight.start, highlight.end));

    let mut cursor = 0usize;
    let mut lines: Vec<Vec<HighlightRun>> = vec![Vec::new()];
    let mut used_highlight = false;

    for highlight in spans {
        if highlight.start < cursor {
            continue;
        }
        push_highlight_segment(&mut lines, &source[cursor..highlight.start], "");
        push_highlight_segment(
            &mut lines,
            &source[highlight.start..highlight.end],
            highlight.style.as_str(),
        );
        used_highlight = true;
        cursor = highlight.end;
    }

    if !used_highlight {
        return Vec::new();
    }

    push_highlight_segment(&mut lines, &source[cursor..], "");
    lines
        .into_iter()
        .map(|runs| HighlightLine { runs })
        .collect()
}

fn push_highlight_segment(lines: &mut Vec<Vec<HighlightRun>>, mut segment: &str, style: &str) {
    while !segment.is_empty() {
        if let Some(newline) = segment.find('\n') {
            push_highlight_run(lines, &segment[..newline], style);
            lines.push(Vec::new());
            segment = &segment[newline + 1..];
        } else {
            push_highlight_run(lines, segment, style);
            break;
        }
    }
}

fn push_highlight_run(lines: &mut [Vec<HighlightRun>], text: &str, style: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = lines.last_mut().and_then(|line| line.last_mut()) {
        if last.style == style {
            last.text.push_str(text);
            return;
        }
    }
    if let Some(line) = lines.last_mut() {
        let start_col = line
            .iter()
            .map(|run| run.text.chars().count())
            .sum::<usize>() as i32;
        line.push(HighlightRun {
            text: text.to_string(),
            style: style.to_string(),
            start_col,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_framework::plugin_ui::{PanelTextHighlight, PanelTextStyle};

    fn collect_lines(
        source: &str,
        spans: &[PanelTextHighlight],
    ) -> Vec<Vec<(String, String, i32)>> {
        build_highlight_lines(source, spans)
            .into_iter()
            .map(|line| {
                line.runs
                    .into_iter()
                    .map(|run| (run.text, run.style, run.start_col))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn invalid_highlight_spans_are_ignored() {
        let lines = build_highlight_lines(
            "alpha",
            &[
                PanelTextHighlight::new(4, 2, PanelTextStyle::Keyword),
                PanelTextHighlight::new(0, 99, PanelTextStyle::String),
            ],
        );

        assert!(lines.is_empty());
    }

    #[test]
    fn gaps_between_highlights_become_plain_runs() {
        let lines = collect_lines(
            "abc def",
            &[PanelTextHighlight::new(4, 7, PanelTextStyle::Keyword)],
        );

        assert_eq!(
            lines,
            vec![vec![
                ("abc ".to_string(), "".to_string(), 0),
                ("def".to_string(), "keyword".to_string(), 4)
            ]]
        );
    }

    #[test]
    fn highlighted_text_is_split_by_lines() {
        let lines = collect_lines(
            "one\ntwo",
            &[PanelTextHighlight::new(0, 3, PanelTextStyle::Function)],
        );

        assert_eq!(
            lines,
            vec![
                vec![("one".to_string(), "function".to_string(), 0)],
                vec![("two".to_string(), "".to_string(), 0)],
            ]
        );
    }

    #[test]
    fn highlighted_text_preserves_empty_lines() {
        let lines = collect_lines(
            "\none\n\n",
            &[PanelTextHighlight::new(1, 4, PanelTextStyle::Function)],
        );

        assert_eq!(
            lines,
            vec![
                Vec::<(String, String, i32)>::new(),
                vec![("one".to_string(), "function".to_string(), 0)],
                Vec::<(String, String, i32)>::new(),
                Vec::<(String, String, i32)>::new(),
            ]
        );
    }

    #[test]
    fn overlapping_highlight_spans_are_ignored() {
        let lines = collect_lines(
            "abcdef",
            &[
                PanelTextHighlight::new(0, 4, PanelTextStyle::String),
                PanelTextHighlight::new(2, 6, PanelTextStyle::Keyword),
            ],
        );

        assert_eq!(
            lines,
            vec![vec![
                ("abcd".to_string(), "string".to_string(), 0),
                ("ef".to_string(), "".to_string(), 4),
            ]]
        );
    }
}
