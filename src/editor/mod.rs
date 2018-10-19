//! Implementation of a distributed editor with a piece table.

use std::cell::RefCell;
use std::cmp;
use std::collections::VecDeque;

mod pt;

use self::pt::PieceTable;

/// One edit in the editor. Each edit happens at a position, which is an index in bytes into the
/// buffer. Edits with an invalid index are rejected. Each edit also has a base revision number,
/// which is used to prevent race conditions.
#[derive(Debug, Serialize, Deserialize)]
pub struct Edit {
    pos: usize,
    base: u32,
    action: EditAction,
}

/// Represents a single editor action, regardless of place.
/// To be used inside Edit.
#[derive(Debug, Serialize, Deserialize)]
pub enum EditAction {
    /// Insert action with offset in bytes, inserted string
    Insert(String),
    /// Delete action with offset and length in bytes
    Delete(usize),
}

pub struct Editor(RefCell<(PieceTable, History)>);

impl Editor {
    pub fn new() -> Self {
        Editor(RefCell::new((PieceTable::new(), History::new())))
    }

    pub fn edit(&self, Edit { pos, base, action }: Edit) -> Result<(), ()> {
        let mut inner = self.0.borrow_mut();
        match action {
            EditAction::Insert(content) => {
                if inner.0.valid_index(pos) {
                    inner.0.insert(pos, &content);
                    Ok(())
                } else {
                    Err(())
                }
            }
            EditAction::Delete(len) => {
                if len > 0 && inner.0.valid_index(pos) && inner.0.valid_index(pos + len) {
                    inner.0.delete(pos, len);
                    Ok(())
                } else {
                    Err(())
                }
            }
        }
    }

    pub fn status(&self) -> (u32, String) {
        let inner = self.0.borrow();
        (inner.1.rev(), inner.0.to_string())
    }
}

struct History {
    first_rev: u32,
    /// Backlog of edits that at least one client has not ack'd.
    /// Pairs of (old offset, new offset).
    /// Example: inserting 5 characters at index 0 generates: (0, 5)
    /// deleting 4 characters at index 6 generates: (10, 6)
    edits: VecDeque<(usize, usize)>,
}

impl History {
    pub fn new() -> Self {
        History {
            first_rev: 0,
            edits: VecDeque::new(),
        }
    }

    /// Reconciles editing race-conditions. If edits happen between the given edit and its
    /// base revision, this function rebases the edit. The return type is a vector because in
    /// certain cases (see below) the edit might need to be split an indeterminate amount of times.
    /// The following interactions might occur:
    ///
    /// * Another editor deleted or inserted a range before the edit;
    ///   in this case, indices need to be adjusted.
    /// * Another editor deleted or inserted a range after the edit;
    ///   in this case, nothing needs to be done
    /// * The edit deletes a range that overlaps with a range deleted by another editor;
    ///   in this case, indices need to be adjusted to avoid deleting an unintended range.
    /// * The edit deletes a range that overlaps with a range inserted by another editor;
    ///   in this case, the edit must be split in two.
    /// * The edit inserts a range contained by a range deleted by another editor;
    ///   in this case, indices are adjusted to move the insert before the deletion (spatially)
    pub fn transform(&self, edit: Edit) -> Result<Edit, &'static str> {
        if edit.base < self.first_rev {
            // The client already knows about a later edit. This is just trolling.
            return Err("old revision");
        }

        let delta = edit.base - self.first_rev;
        let mut pos = edit.pos;

        for &(old, new) in self.edits.iter().skip(delta as usize) {
            if cmp::max(old, new) < pos {
                // Rule 1. Adjust position.
                pos += new - old;
            } else if cmp::min(old, new) > pos {
                // Rule 2. No effect.
                continue;
            } else {
                // some overlap occurs.
                // TODO Implement transform for overlapping ranges.
                return Err("not implemented");
            }
        }

        Ok(Edit { pos, ..edit })
    }

    /// Records the effects of an edit on buffer offsets.
    pub fn record(&mut self, edit: &Edit) {
        self.edits.push_back(match edit.action {
            EditAction::Insert(ref s) => (edit.pos, edit.pos + s.len()),
            EditAction::Delete(len) => (edit.pos + len, edit.pos),
        });
    }

    /// Gets the current revision number
    pub fn rev(&self) -> u32 {
        self.first_rev + self.edits.len() as u32
    }
}
