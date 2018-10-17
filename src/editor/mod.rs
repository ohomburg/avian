//! Implementation of a distributed editor with a piece table.

use std::fmt;
use std::cell::RefCell;
use std::collections::VecDeque;

/// One edit in the editor. Each edit happens at a position, which is an index in bytes into the
/// buffer. Edits with an invalid index are rejected. Each edit also has a base revision number,
/// which is used to prevent race conditions.
#[derive(Debug, Serialize, Deserialize)]
pub struct Edit<'a> {
	pos: usize,
	base: u32,
	#[serde(borrow)]
	action: EditAction<'a>,
}

/// Represents a single editor action, regardless of place.
/// To be used inside Edit.
#[derive(Debug, Serialize, Deserialize)]
pub enum EditAction<'a> {
	/// Insert action with offset in bytes, inserted string
	Insert(&'a str),
	/// Delete action with offset and length in bytes
	Delete(usize),
}

pub struct Editor(RefCell<(PieceTable, History)>);

impl Editor {
	pub fn new() -> Self {
		Editor(RefCell::new((PieceTable::new(), History::new())))
	}

	pub fn edit(&self, Edit { pos, base, action }: Edit) {
		let mut inner = self.0.borrow_mut();
		match action {
			EditAction::Insert(content) => inner.0.insert(pos, content),
			EditAction::Delete(len) => inner.0.delete(pos, len),
		}
		println!("After edit: {}", inner.0);
	}

	pub fn status(&self) -> (u32, String) {
		let inner = self.0.borrow();
		(inner.1.rev(), inner.0.to_string())
	}
}

struct PieceTable {
	/// Editor contents buffer. This only ever grows, unless garbage-collected.
	/// Unlike usual piece-table implementations, this one only uses one buffer.
	/// This simplifies parts of the source code, and does not incur any overhead over having two
	/// strings. Simultaneous insertions can scramble the end of the buffer and generate a lot
	/// of 1-length pieces. In the future, maybe allocate one append buffer per client.
	buffer: String,
	/// Pieces of the actual edit content. Pairs of (offset, length).
	/// Invariant: This is never empty.
	pieces: Vec<(usize, usize)>,
}

impl PieceTable {
	pub fn new() -> Self {
		let init: &[(usize, usize)] = &[(0, 0)];
		PieceTable {
			buffer: String::new(),
			pieces: Vec::from(init),
		}
	}

	/// Checks if pos is in range and on a char boundary.
	pub fn valid_index(&self, pos: usize) -> bool {
		if let Some((piece, len)) = self.piece_index(pos) {
			let offset = self.pieces[piece].1 - (len - pos);
			self.buffer.is_char_boundary(self.pieces[piece].0 + offset)
		} else {
			false
		}
	}

	/// Returns the index of the piece containing string offset pos, and the total length
	/// of all pieces up to that point (inclusive) if pos is in range.
	///
	/// If a piece ends exactly before index pos, it counts as containing it. This is necessary to
	/// ensure the length of the file is a valid index for insertion.
	fn piece_index(&self, pos: usize) -> Option<(usize, usize)> {
		let mut sum = 0;
		for (i, (_, len)) in self.pieces.iter().enumerate() {
			sum += len;
			if sum >= pos {
				return Some((i, sum));
			}
		}
		None
	}

	/// Returns the index of the piece containing string offset pos, and the total length
	/// of all pieces up to that point (inclusive) if pos is in range.
	///
	/// If a piece ends exactly before index pos, it does not count as containing it.
	/// This is the only difference to piece_index.
	fn piece_index_del(&self, pos: usize) -> Option<(usize, usize)> {
		let mut sum = 0;
		for (i, (_, len)) in self.pieces.iter().enumerate() {
			sum += len;
			if sum > pos {
				return Some((i, sum));
			}
		}
		None
	}

	/// Insert text into the editor
	///
	/// Can panic on unwrap if pos is not valid. Use valid_index to check beforehand!
	pub fn insert(&mut self, pos: usize, content: &str) {
		let offset = self.buffer.len();
		self.buffer.push_str(content);

		let (piece, len) = self.piece_index(pos).unwrap();

		let is_end_of_piece = pos == len;
		let is_end_of_buffer = self.pieces[piece].0 + self.pieces[piece].1 == offset;

		// optimized case: if inserting at the end of the previous insertion
		if is_end_of_buffer && is_end_of_piece {
			// just increase the length of the piece
			self.pieces[piece].1 += content.len();
			return;
		}

		let extra_piece = (offset, content.len());
		// optimized case: if inserting at the end of a piece, only need to insert one extra
		if is_end_of_piece {
			self.pieces.insert(piece + 1, extra_piece);
			return;
		}

		// otherwise: split the piece
		let overhead = len - pos;
		self.pieces[piece].1 -= overhead;
		let after_piece = (self.pieces[piece].0 + self.pieces[piece].1, overhead);
		self.pieces.insert(piece + 1, extra_piece);
		self.pieces.insert(piece + 2, after_piece);
	}

	/// Delete text from the editor
	///
	/// Can panic on unwrap if pos is not valid.
	/// Can panic if pos+len is invalid.
	/// Use valid_index to check both beforehand!
	/// Can also panic if all pieces have length zero.
	/// Check this with `len > 0 && valid_index(pos + len)`.
	pub fn delete(&mut self, pos: usize, len: usize) {
		let (piece, end) = self.piece_index_del(pos).unwrap();

		let overlap = pos + len > end;
		let end_of_piece = pos + len == end;
		let start_of_piece = pos == end - self.pieces[piece].1;

		if start_of_piece {
			if end_of_piece {
				// optimized case: deleting an entire piece, no overlap
				self.pieces.remove(piece);
			} else if overlap {
				// optimized case: deleting an entire piece, with overlap
				let (_, piece_len) = self.pieces.remove(piece);
				// recursively delete rest. Same pos, because we just deleted what was there.
				self.delete(pos, len - piece_len);
			} else {
				// optimized case: deleting from the start of a piece, but not until the end
				self.pieces[piece].0 += len;
				self.pieces[piece].1 -= len;
			}
			return;
		}

		// optimized case: deleting from the end of a piece
		if end_of_piece {
			self.pieces[piece].1 -= len;
			return;
		}

		// remaining two cases: either need to recurse, or split the piece
		let overhead = end - pos;
		self.pieces[piece].1 -= overhead;
		if overlap {
			self.delete(pos, len - overhead);
		} else {
			let after_piece = (self.pieces[piece].0 + self.pieces[piece].1 + len, overhead - len);
			self.pieces.insert(piece + 1, after_piece);
		}
	}
}

impl fmt::Display for PieceTable {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for (offset, len) in self.pieces.iter() {
			f.write_str(&self.buffer[*offset..offset + len])?;
		}
		Ok(())
	}
}

impl<T: Into<String>> From<T> for PieceTable {
	fn from(s: T) -> Self {
		let buffer = s.into();
		let init: &[(usize, usize)] = &[(0, buffer.len())];
		PieceTable {
			buffer,
			pieces: Vec::from(init),
		}
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
	pub fn transform(&self, edit: Edit) -> Vec<Edit> {
		unimplemented!()
	}

	/// Records the effects of an edit on buffer offsets.
	pub fn record(&mut self, _edit: &Edit) {
		unimplemented!()
	}

	/// Gets the current revision number
	pub fn rev(&self) -> u32 {
		self.first_rev + self.edits.len() as u32
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn pt_insert() {
		let mut pt = PieceTable::new();
		pt.insert(0, "Hello");
		pt.insert(5, "!");
		assert_eq!("Hello!", pt.to_string());
		pt.insert(5, "World");
		pt.insert(5, " ");
		assert_eq!("Hello World!", pt.to_string());
	}

	#[test]
	fn pt_delete() {
		let mut pt = PieceTable::from("the quick brown fox jumps over the lazy dog");
		pt.delete(3, 1); // remove space before quick
		pt.delete(8, 1); // split between quick and brown
		pt.delete(4, 10); // remove "quick brown"
		pt.delete(0, 4); // remove "the "
		assert_eq!("fox jumps over the lazy dog", pt.to_string());
		// delete all spaces.
		pt.delete(23, 1);
		pt.delete(18, 1);
		pt.delete(14, 1);
		pt.delete(9, 1);
		pt.delete(3, 1);
		// delete entire string, except for "fo" and "g"
		pt.delete(2, 19);
		assert_eq!("fog", pt.to_string());
	}
}
