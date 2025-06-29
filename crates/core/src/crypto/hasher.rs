use std::io::{ErrorKind, Read, Result};

use sha2::{Digest, Sha256};

use crate::id::Id;

/// Hashes the given data.
///
/// # Arguments
///
/// * `data` - The data to hash.
///
/// # Returns
///
/// The hash Id of the data.
#[must_use]
pub fn hash(data: &[u8]) -> Id {
    Id::new(Sha256::digest(data).into())
}

/// Hashes the data from a [`Read`]er.
///
/// # Arguments
///
/// * `reader` - The reader to read the data to hash from.
///
/// # Returns
///
/// # Errors
/// - if the reader encounters an error
///
/// The hash Id of the data.
pub fn hash_reader(mut reader: impl Read) -> Result<Id> {
    let mut buffer = [0; 4096];
    let mut hasher = Sha256::default();

    loop {
        match reader.read(&mut buffer) {
            Err(err) => {
                if err.kind() != ErrorKind::Interrupted {
                    break Err(err);
                }
            }
            Ok(count) => {
                if count == 0 {
                    let id = hasher.finalize();
                    break Ok(Id::new(id.into()));
                }
                hasher.update(&buffer[..count]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn hash_reader_is_identical_to_hash(bytes in prop::collection::vec(prop::num::u8::ANY, 0..65536))  {
            let hash1 = hash(&bytes);
            let hash2 = hash_reader(&*bytes).unwrap();
            prop_assert_eq!(hash1, hash2);
        }
    }
}
