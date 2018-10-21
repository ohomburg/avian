//! Implementation of a distributed editor with a piece table.

extern crate serde;
#[macro_use]
extern crate serde_derive;

use std::cell::RefCell;
use std::cmp;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

mod pt;

use self::pt::PieceTable;

/// One edit in the editor. Each edit happens at a position, which is an index in bytes into the
/// buffer. Edits with an invalid index are rejected. Each edit also has a base revision number,
/// which is used to prevent race conditions.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Edit {
    pub pos: usize,
    /// Base revision when sent by the client, current revision number when sent by the server.
    pub rev: u32,
    pub action: EditAction,
}

/// Represents a single editor action, regardless of place.
/// To be used inside Edit.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum EditAction {
    /// Insert action with offset in bytes, inserted string
    Insert(String),
    /// Delete action with offset and length in bytes
    Delete(usize),
}

/// The main struct to keep track of editor status. Wraps its contents in a RefCell
/// to allow mutation without ownership.
/// The Id is generic for type safety and in case the id type (which is currently always u32)
/// needs to be changed in the future, likely if the ws implementation is switched out.
pub struct Editor<Id>(RefCell<(PieceTable, History, HashMap<Id, u32>)>);

impl<Id: Eq + Hash> Editor<Id> {
    pub fn new() -> Self {
        Editor(RefCell::new((
            PieceTable::new(),
            History::new(),
            HashMap::new(),
        )))
    }

    /// Registers an edit from a specific client.
    /// The edit's rev number is used to determine the client's knowledge,
    /// meaning: the client acknowledges all edits up to number *rev*.
    pub fn edit(&self, id: Id, edit: Edit) -> Result<Edit, &'static str> {
        self.acknowledge(id, edit.rev);
        let mut inner = self.0.borrow_mut();
        let mut edit = inner.1.transform(edit)?;
        match edit.action {
            EditAction::Insert(ref content) => {
                if inner.0.valid_index(edit.pos) {
                    inner.0.insert(edit.pos, content);
                } else {
                    return Err("invalid index");
                }
            }
            EditAction::Delete(len) => {
                if len > 0 && inner.0.valid_index(edit.pos) && inner.0.valid_index(edit.pos + len) {
                    inner.0.delete(edit.pos, len);
                } else {
                    return Err("invalid index");
                }
            }
        }
        inner.1.record(&mut edit);
        Ok(edit)
    }

    /// Signals that a client knows about revision *rev*
    fn acknowledge(&self, id: Id, rev: u32) {
        let mut inner = self.0.borrow_mut();
        inner.2.insert(id, rev);
        let &min_rev = inner.2.values().min().unwrap();
        inner.1.acknowledge(min_rev);
    }

    /// Signals that a client has disconnected
    pub fn disconnect(&self, id: &Id) {
        let mut inner = self.0.borrow_mut();
        inner.2.remove(id);
        let min_opt = inner.2.values().min().map(|&min| min);
        if let Some(min_rev) = min_opt {
            inner.1.acknowledge(min_rev);
        } else {
            let rev = inner.1.rev();
            inner.1.acknowledge(rev);
        }
    }

    /// Adds a client and returns current status
    pub fn connect(&self, id: Id) -> (u32, String) {
        let mut inner = self.0.borrow_mut();
        let rev = inner.1.rev();
        inner.2.insert(id, rev);
        (rev, inner.0.to_string())
    }

    pub fn buffer(&self) -> String {
        self.0.borrow().0.to_string()
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
        if edit.rev < self.first_rev {
            // The client already knows about a later edit. This is just trolling.
            return Err("old revision");
        }
        if edit.rev > self.first_rev + self.edits.len() as u32 {
            return Err("future revision");
        }

        let delta = edit.rev - self.first_rev;
        let mut pos = edit.pos;

        for &(old, new) in self.edits.iter().skip(delta as usize) {
            if old < pos {
                // Rule 1. Adjust position.
                pos += new;
                pos -= old;
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

    /// Records the effects of an edit on buffer offsets. Changes the edit's revision to
    /// the current revision.
    pub fn record(&mut self, edit: &mut Edit) {
        self.edits.push_back(match edit.action {
            EditAction::Insert(ref s) => (edit.pos, edit.pos + s.len()),
            EditAction::Delete(len) => (edit.pos + len, edit.pos),
        });
        edit.rev = self.first_rev + self.edits.len() as u32;
    }

    /// Gets the current revision number
    pub fn rev(&self) -> u32 {
        self.first_rev + self.edits.len() as u32
    }

    /// Removes all backlog entries up to rev
    pub fn acknowledge(&mut self, rev: u32) {
        for _ in self.first_rev..rev {
            self.edits.pop_front();
        }
        self.first_rev = rev;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_client() -> Result<(), &'static str> {
        let editor = Editor::new();
        assert_eq!(editor.connect(0u32), (0, String::new()));
        let edit = Edit {
            rev: 0,
            pos: 0,
            action: EditAction::Insert("This is a test.".to_string()),
        };
        assert_eq!(editor.edit(0, edit)?.rev, 1);
        assert_eq!(editor.buffer(), "This is a test.");
        let edit = Edit {
            rev: 1,
            pos: "This is a te".len(),
            action: EditAction::Delete(1),
        };
        assert_eq!(editor.edit(0, edit)?.rev, 2);
        let edit = Edit {
            rev: 2,
            pos: "This is a te".len(),
            action: EditAction::Insert("x".to_string()),
        };
        assert_eq!(editor.edit(0, edit)?.rev, 3);
        assert_eq!(editor.buffer(), "This is a text.");
        let edit = Edit {
            rev: 3,
            pos: 0,
            action: EditAction::Delete("This is ".len()),
        };
        assert_eq!(editor.edit(0, edit)?.rev, 4);
        assert_eq!(editor.buffer(), "a text.");
        Ok(())
    }

    #[test]
    fn two_clients() {
        let editor = Editor::new();

        assert_eq!(editor.connect(0u32), (0, String::new()));
        let edit = Edit {
            rev: 0,
            pos: 0,
            action: EditAction::Insert("This is a test.".to_string()),
        };
        assert_eq!(editor.edit(0, edit).unwrap().rev, 1);

        assert_eq!(editor.connect(1), (1, "This is a test.".to_string()));

        let edit = Edit {
            rev: 1,
            pos: "This is ".len(),
            action: EditAction::Insert("not ".to_string()),
        };
        assert_eq!(editor.edit(0, edit).unwrap().rev, 2);

        let edit = Edit {
            rev: 1,
            pos: "This is a te".len(),
            action: EditAction::Delete(1),
        };
        assert_eq!(editor.edit(1, edit).unwrap().rev, 3);

        let edit = Edit {
            rev: 3,
            pos: "This is not a te".len(),
            action: EditAction::Insert("x".to_string()),
        };
        assert_eq!(editor.edit(1, edit).unwrap().rev, 4);

        assert_eq!(editor.buffer(), "This is not a text.");

        let edit = Edit {
            rev: 4,
            pos: "This ".len(),
            action: EditAction::Delete("is not a ".len()),
        };
        assert_eq!(editor.edit(0, edit).unwrap().rev, 5);

        let edit = Edit {
            rev: 4,
            pos: "This is not a text.".len(),
            action: EditAction::Insert("\nSo great!".to_string()),
        };
        assert_eq!(editor.edit(1, edit).unwrap().rev, 6);

        assert_eq!(editor.buffer(), "This text.\nSo great!");
    }
}
