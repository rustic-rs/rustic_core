/// In-memory backend to be used for testing
pub mod in_memory_backend {
    use std::{collections::BTreeMap, sync::RwLock};

    use bytes::Bytes;
    use enum_map::EnumMap;

    use rustic_core::{
        ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend,
    };

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

        fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
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

        fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
            Ok(self.0.read().unwrap()[tpe][id].clone())
        }

        fn read_partial(
            &self,
            tpe: FileType,
            id: &Id,
            _cacheable: bool,
            offset: u32,
            length: u32,
        ) -> RusticResult<Bytes> {
            Ok(self.0.read().unwrap()[tpe][id].slice(offset as usize..(offset + length) as usize))
        }

        fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
            // For in-memory backend, return a simple identifier
            // Since this is a testing backend, we can return a formatted path
            let hex_id = id.to_hex();
            match tpe {
                FileType::Config => "config".to_string(),
                FileType::Pack => format!("data/{}/{}", &hex_id[0..2], hex_id.as_str()),
                _ => format!("{}/{}", tpe.dirname(), hex_id.as_str()),
            }
        }
    }

    impl WriteBackend for InMemoryBackend {
        fn create(&self) -> RusticResult<()> {
            Ok(())
        }

        fn write_bytes(
            &self,
            tpe: FileType,
            id: &Id,
            _cacheable: bool,
            buf: Bytes,
        ) -> RusticResult<()> {
            if self.0.write().unwrap()[tpe].insert(*id, buf).is_some() {
                return Err(
                    RusticError::new(ErrorKind::Backend, "ID `{id}` already exists.")
                        .attach_context("id", id.to_string()),
                );
            }

            Ok(())
        }

        fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
            if self.0.write().unwrap()[tpe].remove(id).is_none() {
                return Err(
                    RusticError::new(ErrorKind::Backend, "ID `{id}` does not exist.")
                        .attach_context("id", id.to_string()),
                );
            }
            Ok(())
        }
    }
}
