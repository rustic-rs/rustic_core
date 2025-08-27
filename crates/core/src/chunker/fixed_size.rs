use std::io::Read;

use crate::error::{ErrorKind, RusticError, RusticResult};

/// `ChunkIter` is an iterator that chunks data.
pub(crate) struct ChunkIter<R: Read + Send> {
    /// The reader.
    reader: R,

    /// The size hint is used to optimize memory allocation; this should be an upper bound on the size.
    size_hint: usize,

    /// The size of a chunk.
    size: usize,

    /// If the iterator is finished.
    finished: bool,
}

impl<R: Read + Send> ChunkIter<R> {
    /// Creates a new `ChunkIter`.
    ///
    /// # Arguments
    ///
    /// * `reader` - The reader to read from.
    /// * `size_hint` - The size hint is used to optimize memory allocation; this should be an upper bound on the size.
    /// * `rabin` - The rolling hash.
    pub(crate) fn new(size: usize, reader: R, size_hint: usize) -> Self {
        Self {
            reader,
            size_hint, // size hint is used to optimize memory allocation; this should be an upper bound on the size
            size,
            finished: false,
        }
    }
}

impl<R: Read + Send> Iterator for ChunkIter<R> {
    type Item = RusticResult<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        let mut vec = Vec::with_capacity(self.size_hint.min(self.size));

        let size = match (&mut self.reader)
            .take(self.size as u64)
            .read_to_end(&mut vec)
        {
            Ok(size) => size,
            Err(err) => {
                return Some(Err(RusticError::with_source(
                    ErrorKind::InputOutput,
                    "Failed to read from reader in iterator",
                    err,
                )));
            }
        };

        // If self.min_size is not reached, we are done.
        // Note that the read data is of size size + open_buf_len and self.min_size = minsize + open_buf_len
        if size < self.size {
            self.finished = true;
            vec.truncate(size);
        }
        self.size_hint = self.size_hint.saturating_sub(vec.len()); // size_hint can be too small!
        if vec.is_empty() { None } else { Some(Ok(vec)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::hasher::hash;
    use insta::assert_ron_snapshot;
    use rand::prelude::*;
    use rstest::rstest;
    use std::io::Cursor;

    #[rstest]
    #[case(1024*1024)]
    #[case(1021*1024)]
    fn chunk_random(#[case] chunk_size: usize) {
        const RANDOM_DATA_SIZE: usize = 32 * 1024 * 1024;
        const SEED: u64 = 23;
        let mut rng = StdRng::seed_from_u64(SEED);
        let mut data = vec![0u8; RANDOM_DATA_SIZE];
        rng.fill_bytes(&mut data);
        let mut reader = Cursor::new(data);

        let chunker = ChunkIter::new(chunk_size, &mut reader, 0);
        let chunks: Vec<_> = chunker
            .map(|chunk| {
                let chunk = chunk.unwrap();
                (chunk.len(), hash(&chunk))
            })
            .collect();

        assert_ron_snapshot!(format!("chunk-size{chunk_size}"), chunks);
    }
}
