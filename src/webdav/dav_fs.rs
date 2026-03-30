use crate::webdav::server::WebdavServer;
use crate::webdav::types::WebdavResource;
use bytes::Bytes;
use dav_server::davpath::DavPath;
use dav_server::fs::{
    DavDirEntry, DavFile, DavFileSystem, DavMetaData, FsError, FsFuture, FsResult, FsStream,
    OpenOptions,
};
use futures_util::stream;
use std::sync::Arc;

#[derive(Clone)]
pub struct DavFs {
    server: Arc<WebdavServer>,
}

impl DavFs {
    pub fn new(server: WebdavServer) -> Self {
        Self {
            server: Arc::new(server),
        }
    }
}

impl DavFileSystem for DavFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        _options: OpenOptions,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        let path_str = path.as_url_string();
        let server = self.server.clone();
        Box::pin(async move {
            match server.handle_get(&path_str, None, None).await {
                Ok(response) => Ok(Box::new(DavFileHandle::new(response)) as Box<dyn DavFile>),
                Err(crate::webdav::types::WebdavError::NotFound(_)) => Err(FsError::NotFound),
                Err(_) => Err(FsError::GeneralFailure),
            }
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        _meta: dav_server::fs::ReadDirMeta,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        let path_str = path.as_url_string();
        let server = self.server.clone();
        Box::pin(async move {
            match server.handle_propfind(&path_str).await {
                Ok(resources) => {
                    let entries: Vec<Box<dyn DavDirEntry>> = resources
                        .into_iter()
                        .map(|r| Box::new(DavDirEntryHandle::new(r)) as Box<dyn DavDirEntry>)
                        .collect();
                    Ok(Box::pin(stream::iter(entries)) as FsStream<Box<dyn DavDirEntry>>)
                }
                Err(_) => Err(FsError::GeneralFailure),
            }
        })
    }

    fn metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        let path_str = path.as_url_string();
        let server = self.server.clone();
        Box::pin(async move {
            match server.handle_head(&path_str).await {
                Ok(resource) => Ok(Box::new(DavMetaDataHandle {
                    size: resource.size,
                    is_dir: false,
                }) as Box<dyn DavMetaData>),
                Err(crate::webdav::types::WebdavError::NotFound(_)) => Err(FsError::NotFound),
                Err(_) => Err(FsError::GeneralFailure),
            }
        })
    }
}

#[derive(Debug)]
struct DavFileHandle {
    data: Bytes,
    offset: u64,
}

impl DavFileHandle {
    fn new(response: crate::webdav::server::GetResponse) -> Self {
        Self {
            data: response.bytes,
            offset: 0,
        }
    }
}

impl DavFile for DavFileHandle {
    fn metadata(&mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        let len = self.data.len() as u64;
        Box::pin(async move {
            Ok(Box::new(DavMetaDataHandle {
                size: len,
                is_dir: false,
            }) as Box<dyn DavMetaData>)
        })
    }

    fn write_buf(&mut self, _buf: Box<dyn bytes::Buf + Send>) -> FsFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn write_bytes(&mut self, _buf: Bytes) -> FsFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn read_bytes(&mut self, count: usize) -> FsFuture<'_, Bytes> {
        let offset = self.offset as usize;
        let end = (offset + count).min(self.data.len());
        let result = if offset < self.data.len() {
            self.data.slice(offset..end)
        } else {
            Bytes::new()
        };
        self.offset = end as u64;
        Box::pin(async move { Ok(result) })
    }

    fn seek(&mut self, pos: std::io::SeekFrom) -> FsFuture<'_, u64> {
        let new_offset = match pos {
            std::io::SeekFrom::Start(offset) => offset,
            std::io::SeekFrom::End(offset) => (self.data.len() as i64 + offset).max(0) as u64,
            std::io::SeekFrom::Current(offset) => (self.offset as i64 + offset).max(0) as u64,
        };
        self.offset = new_offset.min(self.data.len() as u64);
        Box::pin(async move { Ok(self.offset) })
    }

    fn flush(&mut self) -> FsFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Clone)]
struct DavMetaDataHandle {
    size: u64,
    is_dir: bool,
}

impl DavMetaData for DavMetaDataHandle {
    fn len(&self) -> u64 {
        self.size
    }

    fn modified(&self) -> FsResult<std::time::SystemTime> {
        Ok(std::time::SystemTime::now())
    }

    fn is_dir(&self) -> bool {
        self.is_dir
    }
}

struct DavDirEntryHandle {
    resource: WebdavResource,
}

impl DavDirEntryHandle {
    fn new(resource: WebdavResource) -> Self {
        Self { resource }
    }
}

impl DavDirEntry for DavDirEntryHandle {
    fn name(&self) -> Vec<u8> {
        self.resource.name.as_bytes().to_vec()
    }

    fn metadata(&self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        let resource = self.resource.clone();
        Box::pin(async move {
            Ok(Box::new(DavMetaDataHandle {
                size: resource.size,
                is_dir: resource.is_dir,
            }) as Box<dyn DavMetaData>)
        })
    }
}
