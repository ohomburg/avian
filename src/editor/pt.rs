use std::fmt;

pub struct PieceTable {
    /// Editor contents buffer. This only ever grows, unless garbage-collected.
    /// Unlike usual piece-table implementations, this one only uses one buffer.
    /// This simplifies parts of the source code, and does not incur any overhead over having two
    /// strings. Simultaneous insertions can scramble the end of the buffer and generate a lot
    /// of 1-length pieces. In the future, maybe allocate one append buffer per client.
    buffer: String,
    /// Pieces of the actual edit content. Pairs of (offset, length).
    /// Invariant: This is never empty.
    /// This is needed because valid_index(0) must always return true.
    /// The invariant can be restored if needed via `self.check_empty()`.
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
            self.empty_check();
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
            self.empty_check();
        } else {
            let after_piece = (
                self.pieces[piece].0 + self.pieces[piece].1 + len,
                overhead - len,
            );
            self.pieces.insert(piece + 1, after_piece);
        }
    }

    /// Checks that self.pieces is not empty. If it is empty, adds a (0, 0) piece.
    fn empty_check(&mut self) {
        if self.pieces.is_empty() {
            self.pieces.push((0, 0));
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

    #[test]
    fn pt_valid_index() {
        assert!(PieceTable::new().valid_index(0));
        let pt = PieceTable::from("Hello!");
        for i in 0..=("Hello!".len()) {
            assert!(pt.valid_index(i));
        }
        assert!(!pt.valid_index(7));
        let pt = PieceTable::from("ä");
        assert_eq!("ä".len(), 2);
        assert!(pt.valid_index(0));
        assert!(pt.valid_index(2));
        assert!(!pt.valid_index(1));
    }
}
