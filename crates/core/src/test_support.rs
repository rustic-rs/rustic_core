use std::{collections::BTreeMap, sync::RwLock};

use bytes::{Bytes, BytesMut};
use enum_map::EnumMap;

use crate::{
    BytesList, ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend,
};

#[derive(Debug, Default)]
pub(crate) struct MemoryBackend {
    files: RwLock<EnumMap<FileType, BTreeMap<Id, Bytes>>>,
}

impl ReadBackend for MemoryBackend {
    fn location(&self) -> String {
        "memory".to_string()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        Ok(self.files.read().unwrap()[tpe]
            .iter()
            .map(|(id, bytes)| (*id, u32::try_from(bytes.len()).unwrap()))
            .collect())
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        self.files.read().unwrap()[tpe]
            .get(id)
            .cloned()
            .ok_or_else(|| {
                RusticError::new(ErrorKind::Backend, "Missing test repository file {id}")
                    .attach_context("id", id.to_string())
            })
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        Ok(self
            .read_full(tpe, id)?
            .slice(offset as usize..(offset + length) as usize))
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        format!("{}/{id}", tpe.dirname())
    }
}

impl WriteBackend for MemoryBackend {
    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        content: BytesList,
    ) -> RusticResult<()> {
        let mut bytes = BytesMut::new();
        for part in content.slice() {
            bytes.extend_from_slice(part);
        }
        _ = self.files.write().unwrap()[tpe].insert(*id, bytes.freeze());
        Ok(())
    }

    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
        assert!(self.files.write().unwrap()[tpe].remove(id).is_some());
        Ok(())
    }
}
