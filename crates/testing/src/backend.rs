/// In-memory backend to be used for testing
pub mod in_memory_backend {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::RwLock,
    };

    use bytes::Bytes;
    use enum_map::EnumMap;

    use rustic_core::{
        ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend,
    };

    #[derive(Debug)]
    /// In-Memory backend to be used for testing
    pub struct InMemoryBackend {
        map: RwLock<EnumMap<FileType, BTreeMap<Id, Bytes>>>,
        is_cold: bool,
        warm: RwLock<EnumMap<FileType, BTreeSet<Id>>>,
    }

    impl Clone for InMemoryBackend {
        fn clone(&self) -> Self {
            let inner_map = self.map.read().unwrap();
            let inner_warm = self.warm.read().unwrap();
            Self {
                map: RwLock::new(EnumMap::from_fn(|tpe| inner_map[tpe].clone())),
                is_cold: self.is_cold,
                warm: RwLock::new(EnumMap::from_fn(|tpe| inner_warm[tpe].clone())),
            }
        }
    }

    impl InMemoryBackend {
        /// Create a new (empty) `InMemoryBackend`
        #[must_use]
        pub fn new() -> Self {
            Self {
                map: RwLock::new(EnumMap::from_fn(|_| BTreeMap::new())),
                is_cold: false,
                warm: RwLock::new(EnumMap::from_fn(|_| BTreeSet::new())),
            }
        }

        /// Create a new (empty) cold `InMemoryBackend`
        #[must_use]
        pub fn new_cold() -> Self {
            Self {
                map: RwLock::new(EnumMap::from_fn(|_| BTreeMap::new())),
                is_cold: true,
                warm: RwLock::new(EnumMap::from_fn(|_| BTreeSet::new())),
            }
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
            Ok(self.map.read().unwrap()[tpe]
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
            if self.is_cold && !self.warm.read().unwrap()[tpe].contains(id) {
                return Err(RusticError::new(
                    ErrorKind::Backend,
                    "tpe {tpe} id `{id}` is not warmed-up",
                )
                .attach_context("tpe", tpe.to_string())
                .attach_context("id", id.to_string()));
            }
            Ok(self.map.read().unwrap()[tpe]
                .get(id)
                .ok_or_else(|| {
                    RusticError::new(
                        ErrorKind::Backend,
                        "Element tpe: {tpe}, id: {id} does not exist in backend",
                    )
                    .attach_context("tpe", tpe.to_string())
                    .attach_context("id", id.to_string())
                })?
                .clone())
        }

        fn read_partial(
            &self,
            tpe: FileType,
            id: &Id,
            _cacheable: bool,
            offset: u32,
            length: u32,
        ) -> RusticResult<Bytes> {
            if self.is_cold && !self.warm.read().unwrap()[tpe].contains(id) {
                return Err(RusticError::new(
                    ErrorKind::Backend,
                    "tpe {tpe} id `{id}` is not warmed-up",
                )
                .attach_context("tpe", tpe.to_string())
                .attach_context("id", id.to_string()));
            }
            Ok(
                self.map.read().unwrap()[tpe][id]
                    .slice(offset as usize..(offset + length) as usize),
            )
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

        fn needs_warm_up(&self) -> bool {
            self.is_cold
        }

        fn warm_up(&self, tpe: FileType, id: &Id) -> RusticResult<()> {
            if self.is_cold {
                _ = self.warm.write().unwrap()[tpe].insert(*id);
            }
            Ok(())
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
            if self.map.write().unwrap()[tpe].insert(*id, buf).is_some() {
                return Err(
                    RusticError::new(ErrorKind::Backend, "ID `{id}` already exists.")
                        .attach_context("id", id.to_string()),
                );
            }

            Ok(())
        }

        fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
            if self.map.write().unwrap()[tpe].remove(id).is_none() {
                return Err(
                    RusticError::new(ErrorKind::Backend, "ID `{id}` does not exist.")
                        .attach_context("id", id.to_string()),
                );
            }
            Ok(())
        }
    }
}
