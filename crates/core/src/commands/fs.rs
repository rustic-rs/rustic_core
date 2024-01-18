//! filesystem commands
use bytes::{Bytes, BytesMut};

use crate::{
    index::ReadIndex,
    repofile::{BlobType, Node},
    repository::{IndexedFull, IndexedTree, Repository},
    Id, RusticResult,
};

/// OpenFile stores all information needed to access the contents of a file node
#[derive(Debug)]
pub struct OpenFile {
    // The list of blobs
    content: Vec<BlobInfo>,
}

// Information about the blob: 1) The id 2) The cumulated sizes of all blobs prior to this one, a.k.a the starting point of this blob.
#[derive(Debug)]
struct BlobInfo {
    id: Id,
    cumsize: usize,
}

impl OpenFile {
    pub(crate) fn from_node<P, S: IndexedFull>(repo: &Repository<P, S>, node: &Node) -> Self {
        let mut start = 0;
        let content = node
            .content
            .as_ref()
            .unwrap()
            .iter()
            .map(|id| {
                let cumsize = start;
                start += repo.index().get_data(id).unwrap().data_length() as usize;
                BlobInfo {
                    id: id.clone(),
                    cumsize,
                }
            })
            .collect();

        Self { content }
    }

    pub(crate) fn read_at<P, S: IndexedFull>(
        &self,
        repo: &Repository<P, S>,
        mut offset: usize,
        mut length: usize,
    ) -> RusticResult<Bytes> {
        // find the start of relevant blobs
        let mut i = self.content.partition_point(|c| c.cumsize <= offset) - 1;
        offset -= self.content[i].cumsize;

        let mut result = BytesMut::with_capacity(length);

        while length > 0 && i < self.content.len() {
            // TODO: We should add some caching here!
            let data =
                repo.index()
                    .blob_from_backend(repo.dbe(), BlobType::Data, &self.content[i].id)?;
            if offset > data.len() {
                // we cannot read behind the blob. This only happens if offset is too large to fit in the last blob
                break;
            }
            let to_copy = (data.len() - offset).min(length);
            result.extend_from_slice(&data[offset..offset + to_copy]);
            offset = 0;
            length -= to_copy;
            i += 1;
        }
        Ok(result.into())
    }
}
