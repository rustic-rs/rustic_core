use std::{
    io::{stdin, Stdin},
    iter::{once, Once},
    path::PathBuf,
};

use crate::{
    backend::{ReadSource, ReadSourceEntry},
    error::RusticResult,
};

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
    /// The open type.
    type Open = Stdin;
    /// The iterator type.
    type Iter = Once<RusticResult<ReadSourceEntry<Stdin>>>;

    /// Returns the size of the source.
    fn size(&self) -> RusticResult<Option<u64>> {
        Ok(None)
    }

    /// Returns an iterator over the source.
    fn entries(&self) -> Self::Iter {
        let open = Some(stdin());
        once(Ok(ReadSourceEntry::from_path(self.path.clone(), open)))
    }
}
