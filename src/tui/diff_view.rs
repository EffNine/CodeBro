#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    pub line_type: DiffLineType,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLineType {
    Context,
    Addition,
    Deletion,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub file: String,
    pub lines: Vec<DiffLine>,
    pub approved: bool,
    pub rejected: bool,
    pub edited: bool,
}

impl FileDiff {
    pub fn parse(file: &str, old_content: &str, new_content: &str) -> Self {
        let mut lines = Vec::new();

        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();

        let max_len = old_lines.len().max(new_lines.len());
        for i in 0..max_len {
            let old_line = old_lines.get(i).copied().unwrap_or("");
            let new_line = new_lines.get(i).copied().unwrap_or("");

            if old_line == new_line {
                lines.push(DiffLine {
                    line_type: DiffLineType::Context,
                    content: old_line.to_string(),
                });
            } else {
                if !old_line.is_empty() {
                    lines.push(DiffLine {
                        line_type: DiffLineType::Deletion,
                        content: old_line.to_string(),
                    });
                }
                if !new_line.is_empty() {
                    lines.push(DiffLine {
                        line_type: DiffLineType::Addition,
                        content: new_line.to_string(),
                    });
                }
            }
        }

        FileDiff {
            file: file.to_string(),
            lines,
            approved: false,
            rejected: false,
            edited: false,
        }
    }

    pub fn addition_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| l.line_type == DiffLineType::Addition)
            .count()
    }

    pub fn deletion_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| l.line_type == DiffLineType::Deletion)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.addition_count() == 0 && self.deletion_count() == 0
    }

    pub fn needs_approval(&self) -> bool {
        self.deletion_count() > 0 || self.addition_count() > 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffAction {
    Accept,
    Reject,
    Edit,
}

#[derive(Debug, Clone)]
pub struct DiffReviewSession {
    pub diffs: Vec<FileDiff>,
    pub current_index: usize,
    pub actions: Vec<DiffAction>,
}

impl DiffReviewSession {
    pub fn new() -> Self {
        DiffReviewSession {
            diffs: Vec::new(),
            current_index: 0,
            actions: Vec::new(),
        }
    }

    pub fn add_diff(&mut self, diff: FileDiff) {
        self.diffs.push(diff);
    }

    pub fn current_diff(&self) -> Option<&FileDiff> {
        self.diffs.get(self.current_index)
    }

    pub fn has_next(&self) -> bool {
        self.current_index < self.diffs.len().saturating_sub(1)
    }

    pub fn has_previous(&self) -> bool {
        self.current_index > 0
    }

    pub fn next(&mut self) {
        if self.has_next() {
            self.current_index += 1;
        }
    }

    pub fn previous(&mut self) {
        if self.has_previous() {
            self.current_index -= 1;
        }
    }

    pub fn apply_action(&mut self, action: DiffAction) -> Result<()> {
        if let Some(diff) = self.diffs.get_mut(self.current_index) {
            match action {
                DiffAction::Accept => {
                    diff.approved = true;
                    diff.rejected = false;
                }
                DiffAction::Reject => {
                    diff.rejected = true;
                    diff.approved = false;
                }
                DiffAction::Edit => {
                    diff.edited = true;
                }
            }
        }
        self.actions.push(action);
        Ok(())
    }

    pub fn accepted_count(&self) -> usize {
        self.diffs.iter().filter(|d| d.approved).count()
    }

    pub fn rejected_count(&self) -> usize {
        self.diffs.iter().filter(|d| d.rejected).count()
    }

    pub fn all_reviewed(&self) -> bool {
        self.diffs
            .iter()
            .all(|d| d.approved || d.rejected || d.edited)
    }

    pub fn approved_diffs(&self) -> Vec<&FileDiff> {
        self.diffs.iter().filter(|d| d.approved).collect()
    }

    pub fn total_additions(&self) -> usize {
        self.diffs.iter().map(|d| d.addition_count()).sum()
    }

    pub fn total_deletions(&self) -> usize {
        self.diffs.iter().map(|d| d.deletion_count()).sum()
    }
}

impl Default for DiffReviewSession {
    fn default() -> Self {
        Self::new()
    }
}

pub fn render_diff_lines(diff: &FileDiff, width: usize) -> Vec<(char, String)> {
    diff.lines
        .iter()
        .map(|line| {
            let (marker, prefix) = match line.line_type {
                DiffLineType::Context => (' ', "  "),
                DiffLineType::Addition => ('+', "  "),
                DiffLineType::Deletion => ('-', "  "),
            };
            let truncated: String = line.content.chars().take(width).collect();
            (marker, format!("{}{}", prefix, truncated))
        })
        .collect()
}

pub fn diff_stats(diff: &FileDiff) -> String {
    format!(
        "{} file: +{} -{}",
        diff.file,
        diff.addition_count(),
        diff.deletion_count()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_parse() {
        let diff = FileDiff::parse("auth.rs", "old line", "new line");
        assert_eq!(diff.deletion_count(), 1);
        assert_eq!(diff.addition_count(), 1);
    }

    #[test]
    fn test_diff_no_changes() {
        let diff = FileDiff::parse("auth.rs", "same", "same");
        assert!(diff.is_empty());
        assert!(!diff.needs_approval());
    }

    #[test]
    fn test_diff_review_accept() {
        let mut session = DiffReviewSession::new();
        session.add_diff(FileDiff::parse("a.rs", "old", "new"));
        session.apply_action(DiffAction::Accept).unwrap();
        assert_eq!(session.accepted_count(), 1);
        assert!(session.all_reviewed());
    }

    #[test]
    fn test_diff_review_reject() {
        let mut session = DiffReviewSession::new();
        session.add_diff(FileDiff::parse("a.rs", "old", "new"));
        session.apply_action(DiffAction::Reject).unwrap();
        assert_eq!(session.rejected_count(), 1);
        assert!(session.all_reviewed());
    }

    #[test]
    fn test_diff_review_navigation() {
        let mut session = DiffReviewSession::new();
        session.add_diff(FileDiff::parse("a.rs", "old", "new"));
        session.add_diff(FileDiff::parse("b.rs", "x", "y"));
        assert!(session.has_next());
        session.next();
        assert_eq!(session.current_diff().unwrap().file, "b.rs");
        assert!(session.has_previous());
        session.previous();
        assert_eq!(session.current_diff().unwrap().file, "a.rs");
    }

    #[test]
    fn test_diff_not_all_reviewed() {
        let mut session = DiffReviewSession::new();
        session.add_diff(FileDiff::parse("a.rs", "old", "new"));
        assert!(!session.all_reviewed());
    }

    #[test]
    fn test_diff_stats() {
        let diff = FileDiff::parse("auth.rs", "old", "new");
        let stats = diff_stats(&diff);
        assert!(stats.contains("auth.rs"));
        assert!(stats.contains("+1"));
        assert!(stats.contains("-1"));
    }
}
