use std::time::Instant;

use crate::buffer::Buffer;

// ---------------------------------------------------------------------------
// Operation — a single atomic text change
// ---------------------------------------------------------------------------

pub enum Operation {
    Insert { pos: usize, text: String },
    Delete { pos: usize, text: String },
}

impl Operation {
    fn apply(&self, buf: &mut Buffer) {
        match self {
            Operation::Insert { pos, text } => buf.insert(*pos, text),
            Operation::Delete { pos, text } => {
                buf.delete(*pos, text.len());
            }
        }
    }

    fn invert(&self) -> Operation {
        match self {
            Operation::Insert { pos, text } => Operation::Delete {
                pos: *pos,
                text: text.clone(),
            },
            Operation::Delete { pos, text } => Operation::Insert {
                pos: *pos,
                text: text.clone(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// CursorState — snapshot of cursor position
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct CursorState {
    pub line: usize,
    pub col: usize,
    pub desired_col: usize,
}

// ---------------------------------------------------------------------------
// GroupContext — what kind of edit created this group
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GroupContext {
    Typing,
    Deleting,
    Paste,
    Cut,
    Other,
}

// ---------------------------------------------------------------------------
// Group — a set of operations that undo/redo together
// ---------------------------------------------------------------------------

struct Group {
    ops: Vec<Operation>,
    cursor_before: CursorState,
    cursor_after: CursorState,
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

const GROUP_TIMEOUT_MS: u128 = 500;

pub struct UndoStack {
    undo: Vec<Group>,
    redo: Vec<Group>,
    pending: Vec<Operation>,
    pending_cursor: Option<CursorState>,
    context: GroupContext,
    last_edit: Option<Instant>,
    saved_at: Option<usize>,
}

impl UndoStack {
    pub fn new() -> Self {
        UndoStack {
            undo: Vec::new(),
            redo: Vec::new(),
            pending: Vec::new(),
            pending_cursor: None,
            context: GroupContext::Other,
            last_edit: None,
            saved_at: Some(0),
        }
    }

    pub fn record(&mut self, op: Operation, cursor_before: CursorState, ctx: GroupContext) {
        // Start a new group if: context changed, timeout elapsed, or pending is empty
        let should_split = self.pending.is_empty()
            || ctx != self.context
            || ctx == GroupContext::Paste
            || ctx == GroupContext::Cut
            || ctx == GroupContext::Other
            || self
                .last_edit
                .is_none_or(|t| t.elapsed().as_millis() >= GROUP_TIMEOUT_MS);

        if should_split && !self.pending.is_empty() {
            // Finish current pending group with cursor_before of the new op as cursor_after
            let ops = std::mem::take(&mut self.pending);
            let group_cursor_before = self.pending_cursor.unwrap_or(cursor_before);
            self.undo.push(Group {
                ops,
                cursor_before: group_cursor_before,
                cursor_after: cursor_before,
            });
        }

        if self.pending.is_empty() {
            self.pending_cursor = Some(cursor_before);
        }

        self.pending.push(op);
        self.context = ctx;
        self.last_edit = Some(Instant::now());

        // Any new edit clears the redo stack
        self.redo.clear();
    }

    pub fn finish_group(&mut self, cursor_after: CursorState) {
        if self.pending.is_empty() {
            return;
        }
        let ops = std::mem::take(&mut self.pending);
        let cursor_before = self.pending_cursor.unwrap_or(cursor_after);
        self.undo.push(Group {
            ops,
            cursor_before,
            cursor_after,
        });
        self.pending_cursor = None;
    }

    pub fn undo(&mut self, buf: &mut Buffer, current_cursor: CursorState) -> Option<CursorState> {
        // Finish any pending group first
        self.finish_group(current_cursor);

        let group = self.undo.pop()?;

        // Apply inverse operations in reverse order
        for op in group.ops.iter().rev() {
            op.invert().apply(buf);
        }

        // Push to redo
        self.redo.push(group);

        let redone = self.redo.last().unwrap();
        Some(redone.cursor_before)
    }

    pub fn redo(&mut self, buf: &mut Buffer) -> Option<CursorState> {
        let group = self.redo.pop()?;

        // Apply operations forward
        for op in &group.ops {
            op.apply(buf);
        }

        let cursor_after = group.cursor_after;

        // Push to undo
        self.undo.push(group);

        Some(cursor_after)
    }

    pub fn mark_saved(&mut self, current_cursor: CursorState) {
        self.finish_group(current_cursor);
        self.saved_at = Some(self.undo.len());
    }

    pub fn is_at_saved(&self) -> bool {
        if !self.pending.is_empty() {
            return false;
        }
        self.saved_at == Some(self.undo.len())
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending.clear();
        self.pending_cursor = None;
        self.last_edit = None;
        self.saved_at = Some(0);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(line: usize, col: usize) -> CursorState {
        CursorState {
            line,
            col,
            desired_col: col,
        }
    }

    #[test]
    fn test_undo_single_insert() {
        let mut buf = Buffer::new();
        let mut stack = UndoStack::new();

        let before = cursor(0, 0);
        buf.insert(0, "hello");
        stack.record(
            Operation::Insert {
                pos: 0,
                text: "hello".to_string(),
            },
            before,
            GroupContext::Paste,
        );

        let after = cursor(0, 5);
        let restored = stack.undo(&mut buf, after);
        assert!(restored.is_some());
        assert_eq!(buf.text(), "");
    }

    #[test]
    fn test_undo_single_delete() {
        let mut buf = Buffer::new();
        buf.insert(0, "hello");
        let mut stack = UndoStack::new();

        let before = cursor(0, 5);
        buf.delete(0, 5);
        stack.record(
            Operation::Delete {
                pos: 0,
                text: "hello".to_string(),
            },
            before,
            GroupContext::Other,
        );

        let after = cursor(0, 0);
        let restored = stack.undo(&mut buf, after);
        assert!(restored.is_some());
        assert_eq!(buf.text(), "hello");
    }

    #[test]
    fn test_redo() {
        let mut buf = Buffer::new();
        let mut stack = UndoStack::new();

        let before = cursor(0, 0);
        buf.insert(0, "hello");
        stack.record(
            Operation::Insert {
                pos: 0,
                text: "hello".to_string(),
            },
            before,
            GroupContext::Paste,
        );

        let after = cursor(0, 5);
        stack.undo(&mut buf, after);
        assert_eq!(buf.text(), "");

        let restored = stack.redo(&mut buf);
        assert!(restored.is_some());
        assert_eq!(buf.text(), "hello");
        assert_eq!(restored.unwrap().col, 5);
    }

    #[test]
    fn test_redo_cleared_on_edit() {
        let mut buf = Buffer::new();
        let mut stack = UndoStack::new();

        let before = cursor(0, 0);
        buf.insert(0, "hello");
        stack.record(
            Operation::Insert {
                pos: 0,
                text: "hello".to_string(),
            },
            before,
            GroupContext::Paste,
        );

        let after = cursor(0, 5);
        stack.undo(&mut buf, after);
        assert_eq!(buf.text(), "");

        // New edit should clear redo
        buf.insert(0, "world");
        stack.record(
            Operation::Insert {
                pos: 0,
                text: "world".to_string(),
            },
            cursor(0, 0),
            GroupContext::Paste,
        );

        let result = stack.redo(&mut buf);
        assert!(result.is_none());
    }

    #[test]
    fn test_grouping_same_context() {
        let mut buf = Buffer::new();
        let mut stack = UndoStack::new();

        // Type 'h', 'e', 'l', 'l', 'o' rapidly (same context, no timeout)
        for (i, ch) in "hello".chars().enumerate() {
            let before = cursor(0, i);
            buf.insert(i, &ch.to_string());
            stack.record(
                Operation::Insert {
                    pos: i,
                    text: ch.to_string(),
                },
                before,
                GroupContext::Typing,
            );
        }

        // All should be one group — single undo removes all
        let after = cursor(0, 5);
        let restored = stack.undo(&mut buf, after);
        assert!(restored.is_some());
        assert_eq!(buf.text(), "");
    }

    #[test]
    fn test_grouping_different_context() {
        let mut buf = Buffer::new();
        let mut stack = UndoStack::new();

        // Type "hi"
        let before = cursor(0, 0);
        buf.insert(0, "h");
        stack.record(
            Operation::Insert {
                pos: 0,
                text: "h".to_string(),
            },
            before,
            GroupContext::Typing,
        );
        buf.insert(1, "i");
        stack.record(
            Operation::Insert {
                pos: 1,
                text: "i".to_string(),
            },
            cursor(0, 1),
            GroupContext::Typing,
        );

        // Paste "world"
        buf.insert(2, "world");
        stack.record(
            Operation::Insert {
                pos: 2,
                text: "world".to_string(),
            },
            cursor(0, 2),
            GroupContext::Paste,
        );

        // Undo paste — only "world" removed
        let after = cursor(0, 7);
        let restored = stack.undo(&mut buf, after);
        assert!(restored.is_some());
        assert_eq!(buf.text(), "hi");

        // Undo typing — "hi" removed
        let restored2 = stack.undo(&mut buf, cursor(0, 2));
        assert!(restored2.is_some());
        assert_eq!(buf.text(), "");
    }

    #[test]
    fn test_cursor_restoration() {
        let mut buf = Buffer::new();
        let mut stack = UndoStack::new();

        let before = cursor(0, 0);
        buf.insert(0, "hello");
        stack.record(
            Operation::Insert {
                pos: 0,
                text: "hello".to_string(),
            },
            before,
            GroupContext::Paste,
        );

        let after = cursor(0, 5);
        let restored = stack.undo(&mut buf, after).unwrap();
        // cursor_before of the group should be returned
        assert_eq!(restored.line, 0);
        assert_eq!(restored.col, 0);
    }

    #[test]
    fn test_saved_position() {
        let mut buf = Buffer::new();
        let mut stack = UndoStack::new();

        assert!(stack.is_at_saved());

        // Insert and mark saved
        buf.insert(0, "hello");
        stack.record(
            Operation::Insert {
                pos: 0,
                text: "hello".to_string(),
            },
            cursor(0, 0),
            GroupContext::Paste,
        );
        stack.mark_saved(cursor(0, 5));
        assert!(stack.is_at_saved());

        // Edit moves away from saved
        buf.insert(5, " world");
        stack.record(
            Operation::Insert {
                pos: 5,
                text: " world".to_string(),
            },
            cursor(0, 5),
            GroupContext::Paste,
        );
        assert!(!stack.is_at_saved());

        // Undo back to saved
        stack.undo(&mut buf, cursor(0, 11));
        assert!(stack.is_at_saved());
    }
}
