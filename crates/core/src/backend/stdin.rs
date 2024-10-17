use std::{
    io::{stdin, Stdin},
    iter::{once, Once},
    path::PathBuf,
};

use crate::backend::{BackendAccessErrorKind, ReadSource, ReadSourceEntry};

/// The `StdinSource` is a `ReadSource` for stdin.
#[derive(Debug, Clone)]
pub struct StdinSource {
    /// The path of the stdin entry.
    path: PathBuf,
}

impl StdinSource {
    /// Creates a new `StdinSource`.
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl ReadSource for StdinSource {
    type Error = BackendAccessErrorKind;
    /// The open type.
    type Open = Stdin;
    /// The iterator type.
    type Iter = Once<Result<ReadSourceEntry<Stdin>, Self::Error>>;

    /// Returns the size of the source.
    fn size(&self) -> Result<Option<u64>, Self::Error> {
        Ok(None)
    }

    /// Returns an iterator over the source.
    fn entries(&self) -> Self::Iter {
        let open = Some(stdin());
        once(ReadSourceEntry::from_path(self.path.clone(), open))
    }
}
