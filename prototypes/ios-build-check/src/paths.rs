//! Stands in for the one call the protocol types make into the root crate's `paths`.

use std::{
    io,
    path::{Path, PathBuf},
};

pub fn canonical(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    dunce::canonicalize(path)
}
