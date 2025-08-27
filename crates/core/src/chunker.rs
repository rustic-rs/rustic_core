use std::io::Read;

mod fixed_size;
pub mod rabin;

use fixed_size::ChunkIter as FixedSizeChunkIter;
use rabin::ChunkIter as RabinChunkIter;
use rustic_cdc::Rabin64;

use crate::{
    RusticResult,
    repofile::{ConfigFile, configfile::Chunker},
};

/// `ChunkIter` is an iterator that chunks data.
pub(crate) enum ChunkIter<R: Read + Send> {
    Rabin(Box<RabinChunkIter<R>>),
    FixedSize(FixedSizeChunkIter<R>),
}

impl<R: Read + Send> ChunkIter<R> {
    pub(crate) fn from_config(
        config: &ConfigFile,
        reader: R,
        size_hint: usize,
    ) -> RusticResult<Self> {
        let iter = match config.chunker() {
            Chunker::Rabin => {
                let poly = config.poly()?;
                let rabin = Rabin64::new_with_polynom(6, &poly);
                Self::Rabin(Box::new(RabinChunkIter::new(
                    rabin,
                    config.chunk_size(),
                    config.chunk_min_size(),
                    config.chunk_max_size(),
                    reader,
                    size_hint,
                )?))
            }
            Chunker::FixedSize => Self::FixedSize(FixedSizeChunkIter::new(
                config.chunk_size(),
                reader,
                size_hint,
            )),
        };
        Ok(iter)
    }
}

impl<R: Read + Send> Iterator for ChunkIter<R> {
    type Item = RusticResult<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Rabin(rabin) => rabin.next(),
            Self::FixedSize(fixed_size) => fixed_size.next(),
        }
    }
}
