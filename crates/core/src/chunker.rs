use std::io::Read;

pub mod rabin;

use rabin::ChunkIter as RabinChunkIter;
use rustic_cdc::Rabin64;

use crate::{RusticResult, repofile::ConfigFile};

/// `ChunkIter` is an iterator that chunks data.
pub(crate) enum ChunkIter<R: Read + Send> {
    Rabin(Box<RabinChunkIter<R>>),
}

impl<R: Read + Send> ChunkIter<R> {
    pub(crate) fn from_config(
        config: &ConfigFile,
        reader: R,
        size_hint: usize,
    ) -> RusticResult<Self> {
        let poly = config.poly()?;
        let rabin = Rabin64::new_with_polynom(6, &poly);
        let iter = Self::Rabin(Box::new(RabinChunkIter::new(reader, size_hint, rabin)));
        Ok(iter)
    }
}

impl<R: Read + Send> Iterator for ChunkIter<R> {
    type Item = RusticResult<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Rabin(rabin) => rabin.next(),
        }
    }
}
