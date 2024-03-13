/// In-memory backend to be used for testing
pub mod in_memory_backend {
    use std::{collections::BTreeMap, sync::RwLock};

    use anyhow::{bail, Result};
    use bytes::Bytes;
    use enum_map::EnumMap;

    use rustic_core::{FileType, Id, ReadBackend, WriteBackend};

    #[derive(Debug)]
    /// In-Memory backend to be used for testing
    pub struct InMemoryBackend(RwLock<EnumMap<FileType, BTreeMap<Id, Bytes>>>);

    impl InMemoryBackend {
        /// Create a new (empty) `InMemoryBackend`
        #[must_use]
        pub fn new() -> Self {
            Self(RwLock::new(EnumMap::from_fn(|_| BTreeMap::new())))
        }
    }

    impl Default for InMemoryBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ReadBackend for InMemoryBackend {
        fn location(&self) -> String {
            "test".to_string()
        }

        fn list_with_size(&self, tpe: FileType) -> Result<Vec<(Id, u32)>> {
            Ok(self.0.read().unwrap()[tpe]
                .iter()
                .map(|(id, byte)| {
                    (
                        *id,
                        u32::try_from(byte.len()).expect("byte length is too large"),
                    )
                })
                .collect())
        }

        fn read_full(&self, tpe: FileType, id: &Id) -> Result<Bytes> {
            Ok(self.0.read().unwrap()[tpe][id].clone())
        }

        fn read_partial(
            &self,
            tpe: FileType,
            id: &Id,
            _cacheable: bool,
            offset: u32,
            length: u32,
        ) -> Result<Bytes> {
            Ok(self.0.read().unwrap()[tpe][id].slice(offset as usize..(offset + length) as usize))
        }
    }

    impl WriteBackend for InMemoryBackend {
        fn create(&self) -> Result<()> {
            Ok(())
        }

        fn write_bytes(&self, tpe: FileType, id: &Id, _cacheable: bool, buf: Bytes) -> Result<()> {
            if self.0.write().unwrap()[tpe].insert(*id, buf).is_some() {
                bail!("id {id} already exists");
            }
            Ok(())
        }

        fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> Result<()> {
            if self.0.write().unwrap()[tpe].remove(id).is_none() {
                bail!("id {id} doesn't exists");
            }
            Ok(())
        }
    }
}
