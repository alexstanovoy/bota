//! How a table of rows that carry their own ids becomes the table indexed by
//! them: rows may be written in any order, and a row added in one place
//! never shifts another's.

/// The rows of `$rows` placed at the index `$at` gives each, in an array of
/// `$count` entries. Fails the build when an index runs past the array or
/// two rows share one; `$count` is the number of rows, so every entry is
/// placed.
macro_rules! placed_by_id {
    ($rows:expr, $count:expr, |$row:ident| $at:expr) => {{
        let rows = $rows;
        assert!(rows.len() == $count);
        let mut every = [rows[0]; $count];
        let mut placed = [false; $count];
        let mut index = 0;
        while index < rows.len() {
            let $row = &rows[index];
            let at: usize = $at;
            assert!(at < $count, "an id runs past its table");
            assert!(!placed[at], "two rows share an id");
            placed[at] = true;
            every[at] = rows[index];
            index += 1;
        }
        every
    }};
}

pub(crate) use placed_by_id;
