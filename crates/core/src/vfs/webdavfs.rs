use std::fmt::{Debug, Formatter};
use std::io::SeekFrom;
#[cfg(not(windows))]
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use crate::repofile::Node;
use crate::repository::{IndexedFull, Repository};
use bytes::{Buf, Bytes};
use futures::FutureExt;

use dav_server::davpath::DavPath;
use dav_server::fs::*;

use super::{FilePolicy, OpenFile, Vfs};

fn now() -> SystemTime {
    static NOW: OnceLock<SystemTime> = OnceLock::new();
    *NOW.get_or_init(SystemTime::now)
}

// TODO: add blocking() to operation which may block!!
/*
#[derive(Clone, Copy)]
enum RuntimeType {
    Basic,
    ThreadPool,
}

impl RuntimeType {
    fn get() -> Self {
        static RUNTIME_TYPE: OnceLock<RuntimeType> = OnceLock::new();
        *RUNTIME_TYPE.get_or_init(|| {
            let dbg = format!("{:?}", tokio::runtime::Handle::current());
            if dbg.contains("ThreadPool") {
                Self::ThreadPool
            } else {
                Self::Basic
            }
        })
    }
}

// Run some code via block_in_place() or spawn_blocking().
async fn blocking<F, R>(func: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    match RuntimeType::get() {
        RuntimeType::Basic => tokio::task::spawn_blocking(func).await.unwrap(),
        RuntimeType::ThreadPool => tokio::task::block_in_place(func),
    }
}
*/

/// DAV Filesystem implementation.
#[derive(Debug)]
pub struct WebDavFS<P, S> {
    inner: Arc<DavFsInner<P, S>>,
}

// inner struct.
struct DavFsInner<P, S> {
    repo: Repository<P, S>,
    vfs: Vfs,
}
impl<P, S> Debug for DavFsInner<P, S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "DavFS")
    }
}

struct DavFsFile<P, S> {
    node: Node,
    open: OpenFile,
    fs: Arc<DavFsInner<P, S>>,
    seek: usize,
}
impl<P, S> Debug for DavFsFile<P, S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "DavFile")
    }
}

struct DavFsDirEntry(Node);
#[derive(Clone, Debug)]
struct DavFsMetaData(Node);

impl<P, S: IndexedFull> WebDavFS<P, S> {
    pub(crate) fn new(repo: Repository<P, S>, root: Vfs) -> Box<Self> {
        let inner = DavFsInner { repo, vfs: root };
        Box::new({
            Self {
                inner: Arc::new(inner),
            }
        })
    }

    fn node_from_path(&self, path: &DavPath) -> Result<Node, FsError> {
        self.inner
            .vfs
            .node_from_path(&self.inner.repo, &path.as_pathbuf())
            .map_err(|_| FsError::GeneralFailure)
    }

    fn dir_entries_from_path(&self, path: &DavPath) -> Result<Vec<Node>, FsError> {
        self.inner
            .vfs
            .dir_entries_from_path(&self.inner.repo, &path.as_pathbuf())
            .map_err(|_| FsError::GeneralFailure)
    }
}

impl<P, S: IndexedFull> Clone for WebDavFS<P, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
// This implementation is basically a bunch of boilerplate to
// wrap the std::fs call in self.blocking() calls.
impl<P: Debug + Send + Sync + 'static, S: IndexedFull + Debug + Send + Sync + 'static> DavFileSystem
    for WebDavFS<P, S>
{
    fn metadata<'a>(&'a self, davpath: &'a DavPath) -> FsFuture<'_, Box<dyn DavMetaData>> {
        self.symlink_metadata(davpath)
    }

    fn symlink_metadata<'a>(&'a self, davpath: &'a DavPath) -> FsFuture<'_, Box<dyn DavMetaData>> {
        async move {
            let node = self.node_from_path(davpath)?;
            let meta: Box<dyn DavMetaData> = Box::new(DavFsMetaData(node));
            Ok(meta)
        }
        .boxed()
    }

    // read_dir is a bit more involved - but not much - than a simple wrapper,
    // because it returns a stream.
    fn read_dir<'a>(
        &'a self,
        davpath: &'a DavPath,
        _meta: ReadDirMeta,
    ) -> FsFuture<'_, FsStream<Box<dyn DavDirEntry>>> {
        async move {
            let entries = self.dir_entries_from_path(davpath)?;
            let entry_iter = entries.into_iter().map(|e| {
                let entry: Box<dyn DavDirEntry> = Box::new(DavFsDirEntry(e));
                entry
            });
            let strm: FsStream<Box<dyn DavDirEntry>> = Box::pin(futures::stream::iter(entry_iter));
            Ok(strm)
        }
        .boxed()
    }

    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
    ) -> FsFuture<'_, Box<dyn DavFile>> {
        async move {
            if options.write
                || options.append
                || options.truncate
                || options.create
                || options.create_new
            {
                return Err(FsError::Forbidden);
            }

            let node = self.node_from_path(path)?;
            if let FilePolicy::Forbidden = self.inner.vfs.file_policy {
                return Err(FsError::Forbidden);
            }

            let open = self
                .inner
                .repo
                .open_file(&node)
                .map_err(|_| FsError::GeneralFailure)?;
            let file: Box<dyn DavFile> = Box::new(DavFsFile {
                node,
                open,
                fs: self.inner.clone(),
                seek: 0,
            });
            Ok(file)
        }
        .boxed()
    }
}

impl DavDirEntry for DavFsDirEntry {
    fn metadata(&self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        async move {
            let meta: Box<dyn DavMetaData> = Box::new(DavFsMetaData(self.0.clone()));
            Ok(meta)
        }
        .boxed()
    }

    #[cfg(not(windows))]
    fn name(&self) -> Vec<u8> {
        self.0.name().as_bytes().to_vec()
    }

    #[cfg(windows)]
    fn name(&self) -> Vec<u8> {
        self.0
            .name()
            .as_os_str()
            .to_string_lossy()
            .to_string()
            .into_bytes()
    }
}

impl<P: Debug + Send + Sync, S: IndexedFull + Debug + Send + Sync> DavFile for DavFsFile<P, S> {
    fn metadata(&mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        async move {
            let meta: Box<dyn DavMetaData> = Box::new(DavFsMetaData(self.node.clone()));
            Ok(meta)
        }
        .boxed()
    }

    fn write_bytes(&mut self, _buf: Bytes) -> FsFuture<'_, ()> {
        async move { Err(FsError::Forbidden) }.boxed()
    }

    fn write_buf(&mut self, _buf: Box<dyn Buf + Send>) -> FsFuture<'_, ()> {
        async move { Err(FsError::Forbidden) }.boxed()
    }

    fn read_bytes(&mut self, count: usize) -> FsFuture<'_, Bytes> {
        async move {
            let data = self
                .fs
                .repo
                .read_file_at(&self.open, self.seek, count)
                .map_err(|_| FsError::GeneralFailure)?;
            Ok(data)
        }
        .boxed()
    }

    fn seek(&mut self, pos: SeekFrom) -> FsFuture<'_, u64> {
        async move {
            match pos {
                SeekFrom::Start(start) => self.seek = start as usize,
                SeekFrom::Current(delta) => self.seek = (self.seek as i64 + delta) as usize,
                SeekFrom::End(end) => self.seek = (self.node.meta.size as i64 + end) as usize,
            }

            Ok(self.seek as u64)
        }
        .boxed()
    }

    fn flush(&mut self) -> FsFuture<'_, ()> {
        async move { Ok(()) }.boxed()
    }
}

impl DavMetaData for DavFsMetaData {
    fn len(&self) -> u64 {
        self.0.meta.size
    }
    fn created(&self) -> FsResult<SystemTime> {
        Ok(now())
    }
    fn modified(&self) -> FsResult<SystemTime> {
        Ok(self.0.meta.mtime.map(SystemTime::from).unwrap_or_else(now))
    }
    fn accessed(&self) -> FsResult<SystemTime> {
        Ok(self.0.meta.atime.map(SystemTime::from).unwrap_or_else(now))
    }

    fn status_changed(&self) -> FsResult<SystemTime> {
        Ok(self.0.meta.ctime.map(SystemTime::from).unwrap_or_else(now))
    }

    fn is_dir(&self) -> bool {
        self.0.is_dir()
    }
    fn is_file(&self) -> bool {
        self.0.is_file()
    }
    fn is_symlink(&self) -> bool {
        self.0.is_symlink()
    }
    fn executable(&self) -> FsResult<bool> {
        if self.0.is_file() {
            let Some(mode) = self.0.meta.mode else {
                return Ok(false);
            };
            return Ok((mode & 0o100) > 0);
        }
        Err(FsError::NotImplemented)
    }
}
