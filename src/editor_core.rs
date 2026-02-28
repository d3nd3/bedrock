use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub start: usize,
    pub end: usize,
}

impl Selection {
    pub fn new(start: usize, end: usize) -> Self {
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }

    pub fn cursor(pos: usize) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }

    pub fn is_cursor(self) -> bool {
        self.start == self.end
    }

    pub fn clamp(self, len: usize) -> Self {
        Self::new(self.start.min(len), self.end.min(len))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChange {
    pub start: usize,
    pub end: usize,
    pub insert: String,
}

impl TextChange {
    pub fn new(start: usize, end: usize, insert: impl Into<String>) -> Self {
        Self {
            start,
            end,
            insert: insert.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum ChangeOrigin {
    Input,
    Command,
    Plugin,
    #[default]
    System,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub changes: Vec<TextChange>,
    pub selection_after: Option<Selection>,
    pub origin: ChangeOrigin,
    pub label: &'static str,
}

impl Transaction {
    pub fn single(
        change: TextChange,
        selection_after: Option<Selection>,
        origin: ChangeOrigin,
        label: &'static str,
    ) -> Self {
        Self {
            changes: vec![change],
            selection_after,
            origin,
            label,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub text_changed: bool,
    pub selection_changed: bool,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreError {
    InvalidRange {
        start: usize,
        end: usize,
        len: usize,
    },
    OverlappingChanges {
        first_start: usize,
        first_end: usize,
        next_start: usize,
        next_end: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorSnapshot {
    pub text: String,
    pub selection: Selection,
    pub revision: u64,
}

impl EditorSnapshot {
    pub fn new(text: String) -> Self {
        let len = text.len();
        Self {
            text,
            selection: Selection::cursor(len),
            revision: 0,
        }
    }

    pub fn set_selection(&mut self, selection: Selection) {
        self.selection = selection.clamp(self.text.len());
    }

    pub fn replace_from_input(&mut self, new_text: String, selection: Selection) -> ApplyOutcome {
        let next_selection = selection.clamp(new_text.len());
        let text_changed = self.text != new_text;
        let selection_changed = self.selection != next_selection;

        self.text = new_text;
        self.selection = next_selection;
        if text_changed {
            self.revision += 1;
        }

        ApplyOutcome {
            text_changed,
            selection_changed,
            revision: self.revision,
        }
    }

    pub fn apply_transaction(
        &mut self,
        transaction: Transaction,
    ) -> Result<ApplyOutcome, CoreError> {
        let normalized = normalize_changes(&transaction.changes, self.text.len())?;
        let next_text = if normalized.is_empty() {
            self.text.clone()
        } else {
            apply_changes_to_text(&self.text, &normalized)
        };

        let next_selection = transaction
            .selection_after
            .map(|selection| selection.clamp(next_text.len()))
            .unwrap_or_else(|| {
                Selection::new(
                    map_position_through_changes(self.selection.start, &normalized),
                    map_position_through_changes(self.selection.end, &normalized),
                )
                .clamp(next_text.len())
            });

        let text_changed = self.text != next_text;
        let selection_changed = self.selection != next_selection;

        self.text = next_text;
        self.selection = next_selection;
        if text_changed {
            self.revision += 1;
        }

        Ok(ApplyOutcome {
            text_changed,
            selection_changed,
            revision: self.revision,
        })
    }
}

fn normalize_changes(changes: &[TextChange], len: usize) -> Result<Vec<TextChange>, CoreError> {
    let mut sorted = changes.to_vec();
    sorted.sort_by_key(|change| (change.start, change.end));

    for change in &sorted {
        if change.start > change.end || change.end > len {
            return Err(CoreError::InvalidRange {
                start: change.start,
                end: change.end,
                len,
            });
        }
    }

    for pair in sorted.windows(2) {
        let first = &pair[0];
        let next = &pair[1];
        if next.start < first.end {
            return Err(CoreError::OverlappingChanges {
                first_start: first.start,
                first_end: first.end,
                next_start: next.start,
                next_end: next.end,
            });
        }
    }

    Ok(sorted)
}

fn apply_changes_to_text(text: &str, changes: &[TextChange]) -> String {
    let mut out = String::new();
    let mut cursor = 0usize;
    for change in changes {
        out.push_str(&text[cursor..change.start]);
        out.push_str(&change.insert);
        cursor = change.end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn map_position_through_changes(mut pos: usize, changes: &[TextChange]) -> usize {
    for change in changes {
        if pos < change.start {
            continue;
        }
        if pos <= change.end {
            pos = change.start + change.insert.len();
            continue;
        }
        let removed = change.end - change.start;
        if change.insert.len() >= removed {
            pos += change.insert.len() - removed;
        } else {
            pos = pos.saturating_sub(removed - change.insert.len());
        }
    }
    pos
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownCommand {
    Wrap {
        open: &'static str,
        close: &'static str,
        label: &'static str,
    },
    PrefixLine {
        prefix: &'static str,
        label: &'static str,
    },
    Indent,
    Outdent,
    ContinueBlock,
    AutoPair {
        open: &'static str,
        close: &'static str,
    },
}

pub fn apply_markdown_command(
    snapshot: &mut EditorSnapshot,
    command: MarkdownCommand,
) -> Result<bool, CoreError> {
    let Some(transaction) = build_markdown_transaction(snapshot, command) else {
        return Ok(false);
    };
    let outcome = snapshot.apply_transaction(transaction)?;
    Ok(outcome.text_changed || outcome.selection_changed)
}

fn build_markdown_transaction(
    snapshot: &EditorSnapshot,
    command: MarkdownCommand,
) -> Option<Transaction> {
    match command {
        MarkdownCommand::Wrap { open, close, label } => {
            Some(wrap_transaction(snapshot, open, close, label))
        }
        MarkdownCommand::PrefixLine { prefix, label } => {
            Some(prefix_line_transaction(snapshot, prefix, label))
        }
        MarkdownCommand::Indent => indent_or_outdent_transaction(snapshot, false),
        MarkdownCommand::Outdent => indent_or_outdent_transaction(snapshot, true),
        MarkdownCommand::ContinueBlock => continue_markdown_block_transaction(snapshot),
        MarkdownCommand::AutoPair { open, close } => {
            Some(wrap_transaction(snapshot, open, close, "autopair"))
        }
    }
}

fn wrap_transaction(
    snapshot: &EditorSnapshot,
    open: &str,
    close: &str,
    label: &'static str,
) -> Transaction {
    let selection = snapshot.selection.clamp(snapshot.text.len());
    let mut insert = String::new();
    insert.push_str(open);
    insert.push_str(&snapshot.text[selection.start..selection.end]);
    insert.push_str(close);
    let selection_after = if selection.is_cursor() {
        Selection::cursor(selection.start + open.len())
    } else {
        // For wrapped selections, collapse caret after the closing token.
        // This avoids keeping an invisible selection range in the transparent textarea layer.
        Selection::cursor(selection.end + open.len() + close.len())
    };
    Transaction::single(
        TextChange::new(selection.start, selection.end, insert),
        Some(selection_after),
        ChangeOrigin::Command,
        label,
    )
}

fn prefix_line_transaction(
    snapshot: &EditorSnapshot,
    prefix: &str,
    label: &'static str,
) -> Transaction {
    let selection = snapshot.selection.clamp(snapshot.text.len());
    let start = line_start(&snapshot.text, selection.start);
    let selection_after =
        Selection::new(selection.start + prefix.len(), selection.end + prefix.len());
    Transaction::single(
        TextChange::new(start, start, prefix),
        Some(selection_after),
        ChangeOrigin::Command,
        label,
    )
}

fn indent_or_outdent_transaction(snapshot: &EditorSnapshot, outdent: bool) -> Option<Transaction> {
    let text = &snapshot.text;
    let selection = snapshot.selection.clamp(text.len());

    if selection.is_cursor() {
        if !outdent {
            return Some(Transaction::single(
                TextChange::new(selection.start, selection.end, "    "),
                Some(Selection::cursor(selection.start + 4)),
                ChangeOrigin::Command,
                "indent",
            ));
        }

        let ls = line_start(text, selection.start);
        let le = line_end(text, selection.start);
        let line = &text[ls..le];
        let remove = if line.starts_with('\t') {
            1
        } else {
            line.chars().take_while(|c| *c == ' ').take(4).count()
        };
        if remove == 0 {
            return None;
        }
        let mut replaced = String::new();
        replaced.push_str(&line[remove..]);

        let cursor_offset = selection.start.saturating_sub(ls);
        let new_cursor = if cursor_offset >= remove {
            selection.start - remove
        } else {
            ls
        };

        return Some(Transaction::single(
            TextChange::new(ls, le, replaced),
            Some(Selection::cursor(new_cursor)),
            ChangeOrigin::Command,
            "outdent",
        ));
    }

    let block_start = line_start(text, selection.start);
    let block_end = line_end(text, selection.end);
    let block = &text[block_start..block_end];
    let mut transformed = String::new();

    for (idx, line) in block.split('\n').enumerate() {
        if idx > 0 {
            transformed.push('\n');
        }
        if outdent {
            if line.starts_with('\t') {
                transformed.push_str(&line[1..]);
            } else {
                let remove = line.chars().take_while(|c| *c == ' ').take(4).count();
                transformed.push_str(&line[remove..]);
            }
        } else {
            transformed.push_str("    ");
            transformed.push_str(line);
        }
    }

    Some(Transaction::single(
        TextChange::new(block_start, block_end, transformed.clone()),
        Some(Selection::new(block_start, block_start + transformed.len())),
        ChangeOrigin::Command,
        if outdent {
            "outdent-block"
        } else {
            "indent-block"
        },
    ))
}

fn continue_markdown_block_transaction(snapshot: &EditorSnapshot) -> Option<Transaction> {
    let text = &snapshot.text;
    let selection = snapshot.selection.clamp(text.len());
    if !selection.is_cursor() {
        return None;
    }

    static RE_TASK: OnceLock<Regex> = OnceLock::new();
    static RE_UL: OnceLock<Regex> = OnceLock::new();
    static RE_OL: OnceLock<Regex> = OnceLock::new();
    static RE_QUOTE: OnceLock<Regex> = OnceLock::new();

    let re_task =
        RE_TASK.get_or_init(|| Regex::new(r"^(\s*[-*+]\s+)\[(?: |x|X)\]\s+(.*)$").unwrap());
    let re_ul = RE_UL.get_or_init(|| Regex::new(r"^(\s*[-*+]\s+)(.*)$").unwrap());
    let re_ol = RE_OL.get_or_init(|| Regex::new(r"^(\s*)(\d+)\.\s+(.*)$").unwrap());
    let re_quote = RE_QUOTE.get_or_init(|| Regex::new(r"^(\s*>\s+)(.*)$").unwrap());

    let ls = line_start(text, selection.start);
    let le = line_end(text, selection.start);
    let line = &text[ls..le];

    let insert = if let Some(cap) = re_task.captures(line) {
        let body = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
        if body.trim().is_empty() {
            "\n".to_string()
        } else {
            let prefix = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
            format!("\n{prefix}[ ] ")
        }
    } else if let Some(cap) = re_ol.captures(line) {
        let body = cap.get(3).map(|m| m.as_str()).unwrap_or_default();
        if body.trim().is_empty() {
            "\n".to_string()
        } else {
            let indent = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
            let current = cap
                .get(2)
                .map(|m| m.as_str())
                .unwrap_or("1")
                .parse::<u64>()
                .unwrap_or(1);
            format!("\n{indent}{}. ", current + 1)
        }
    } else if let Some(cap) = re_ul.captures(line) {
        let body = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
        if body.trim().is_empty() {
            "\n".to_string()
        } else {
            let prefix = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
            format!("\n{prefix}")
        }
    } else if let Some(cap) = re_quote.captures(line) {
        let body = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
        if body.trim().is_empty() {
            "\n".to_string()
        } else {
            let prefix = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
            format!("\n{prefix}")
        }
    } else {
        return None;
    };

    let next_cursor = selection.start + insert.len();
    Some(Transaction::single(
        TextChange::new(selection.start, selection.end, insert),
        Some(Selection::cursor(next_cursor)),
        ChangeOrigin::Command,
        "continue-markdown-block",
    ))
}

fn line_start(text: &str, pos: usize) -> usize {
    let clamped = pos.min(text.len());
    text[..clamped].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn line_end(text: &str, pos: usize) -> usize {
    let clamped = pos.min(text.len());
    text[clamped..]
        .find('\n')
        .map(|i| clamped + i)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_multi_change_transaction() {
        let mut snapshot = EditorSnapshot::new("hello world".to_string());
        snapshot.set_selection(Selection::cursor(0));
        let transaction = Transaction {
            changes: vec![TextChange::new(0, 0, ">>"), TextChange::new(11, 11, "<<")],
            selection_after: Some(Selection::cursor(13)),
            origin: ChangeOrigin::Command,
            label: "wrap",
        };

        let outcome = snapshot.apply_transaction(transaction).unwrap();
        assert!(outcome.text_changed);
        assert_eq!(snapshot.text, ">>hello world<<");
        assert_eq!(snapshot.selection, Selection::cursor(13));
        assert_eq!(snapshot.revision, 1);
    }

    #[test]
    fn rejects_overlapping_changes() {
        let mut snapshot = EditorSnapshot::new("abcdef".to_string());
        let transaction = Transaction {
            changes: vec![TextChange::new(1, 4, "x"), TextChange::new(3, 5, "y")],
            selection_after: None,
            origin: ChangeOrigin::Command,
            label: "bad",
        };
        assert!(matches!(
            snapshot.apply_transaction(transaction),
            Err(CoreError::OverlappingChanges { .. })
        ));
    }

    #[test]
    fn wraps_selection_with_markdown() {
        let mut snapshot = EditorSnapshot::new("bedrock".to_string());
        snapshot.set_selection(Selection::new(0, 7));
        let changed = apply_markdown_command(
            &mut snapshot,
            MarkdownCommand::Wrap {
                open: "**",
                close: "**",
                label: "bold",
            },
        )
        .unwrap();

        assert!(changed);
        assert_eq!(snapshot.text, "**bedrock**");
        assert_eq!(snapshot.selection, Selection::cursor(11));
    }

    #[test]
    fn continues_unordered_list() {
        let mut snapshot = EditorSnapshot::new("- item".to_string());
        snapshot.set_selection(Selection::cursor(snapshot.text.len()));
        let changed =
            apply_markdown_command(&mut snapshot, MarkdownCommand::ContinueBlock).unwrap();
        assert!(changed);
        assert_eq!(snapshot.text, "- item\n- ");
    }

    #[test]
    fn indents_and_outdents_block() {
        let mut snapshot = EditorSnapshot::new("a\nb".to_string());
        snapshot.set_selection(Selection::new(0, snapshot.text.len()));
        apply_markdown_command(&mut snapshot, MarkdownCommand::Indent).unwrap();
        assert_eq!(snapshot.text, "    a\n    b");

        snapshot.set_selection(Selection::new(0, snapshot.text.len()));
        apply_markdown_command(&mut snapshot, MarkdownCommand::Outdent).unwrap();
        assert_eq!(snapshot.text, "a\nb");
    }
}
