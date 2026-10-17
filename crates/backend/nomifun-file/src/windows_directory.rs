//! The ordinary file owner and administrator read worker use identical native
//! relative opens and retained directory cursors, under their own OS tokens.
pub(crate) use nomi_native_files::windows::{
    open, open_delete, open_metadata, open_cursor, open_read_cursor, Entries,
};