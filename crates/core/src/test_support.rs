use std::{
    collections::BTreeMap,
    io::Cursor,
    path::Path,
    sync::{
        RwLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::{Bytes, BytesMut};
use enum_map::EnumMap;

use crate::{
    BytesList, ErrorKind, FileType, Id, ReadBackend, ReadSource, ReadSourceEntry, RusticError,
    RusticResult, WriteBackend,
    backend::node::{Metadata, Node, NodeType},
};

#[derive(Debug, Default)]
pub(crate) struct MemoryBackend {
    files: RwLock<EnumMap<FileType, BTreeMap<Id, Bytes>>>,
    write_failure: RwLock<Option<FileType>>,
    failed_writes: AtomicUsize,
}

impl MemoryBackend {
    pub(crate) fn fail_writes(&self, tpe: FileType) {
        *self.write_failure.write().unwrap() = Some(tpe);
    }

    pub(crate) fn failed_writes(&self) -> usize {
        self.failed_writes.load(Ordering::SeqCst)
    }
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
        if *self.write_failure.read().unwrap() == Some(tpe) {
            _ = self.failed_writes.fetch_add(1, Ordering::SeqCst);
            return Err(RusticError::new(
                ErrorKind::Backend,
                "Injected test write failure",
            ));
        }
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

pub(crate) struct MemorySource {
    pub(crate) files: Vec<(&'static str, &'static [u8])>,
}

impl ReadSource for MemorySource {
    type Open = Cursor<&'static [u8]>;
    type Iter = std::vec::IntoIter<RusticResult<ReadSourceEntry<Self::Open>>>;

    fn size(&self) -> RusticResult<Option<u64>> {
        Ok(Some(
            self.files
                .iter()
                .map(|(_, bytes)| bytes.len() as u64)
                .sum(),
        ))
    }

    fn entries(&self) -> Self::Iter {
        self.files
            .iter()
            .map(|(name, bytes)| {
                let path = Path::new("source").join(name);
                let node = Node::new_node(
                    path.file_name().unwrap(),
                    NodeType::File,
                    Metadata {
                        size: bytes.len() as u64,
                        ..Default::default()
                    },
                );
                Ok(ReadSourceEntry {
                    path,
                    node,
                    open: Some(Cursor::new(*bytes)),
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}
