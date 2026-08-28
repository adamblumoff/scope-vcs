use crate::{MultipartError, MultipartStore, MultipartUpload, RemoteReader, UploadedPart};
use async_trait::async_trait;
use bytes::Bytes;
use std::{collections::HashMap, sync::Mutex};
use tokio::io::AsyncWriteExt;

#[derive(Default)]
pub struct MemoryMultipartStore {
    state: Mutex<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    next_upload_id: u64,
    uploads: HashMap<String, MemoryUpload>,
    objects: HashMap<String, Bytes>,
}

struct MemoryUpload {
    key: String,
    parts: HashMap<i32, Bytes>,
}

impl MemoryMultipartStore {
    pub fn object(&self, key: &str) -> Option<Bytes> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .get(key)
            .cloned()
    }
}

#[async_trait]
impl MultipartStore for MemoryMultipartStore {
    async fn begin(&self, key: &str) -> Result<MultipartUpload, MultipartError> {
        let mut state = self.state.lock().expect("memory multipart store lock");
        state.next_upload_id += 1;
        let upload_id = state.next_upload_id.to_string();
        state.uploads.insert(
            upload_id.clone(),
            MemoryUpload {
                key: key.to_string(),
                parts: HashMap::new(),
            },
        );
        Ok(MultipartUpload {
            key: key.to_string(),
            upload_id,
        })
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        bytes: Bytes,
    ) -> Result<UploadedPart, MultipartError> {
        let mut state = self.state.lock().expect("memory multipart store lock");
        let pending = state
            .uploads
            .get_mut(&upload.upload_id)
            .ok_or_else(|| MultipartError::new("multipart upload does not exist"))?;
        if pending.key != upload.key {
            return Err(MultipartError::new("multipart upload key does not match"));
        }
        pending.parts.insert(part_number, bytes);
        Ok(UploadedPart {
            part_number,
            etag: format!("memory-{part_number}"),
        })
    }

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), MultipartError> {
        let mut state = self.state.lock().expect("memory multipart store lock");
        let mut pending = state
            .uploads
            .remove(&upload.upload_id)
            .ok_or_else(|| MultipartError::new("multipart upload does not exist"))?;
        let mut object = Vec::new();
        for part in parts {
            let bytes = pending
                .parts
                .remove(&part.part_number)
                .ok_or_else(|| MultipartError::new("multipart part does not exist"))?;
            object.extend_from_slice(&bytes);
        }
        state.objects.insert(upload.key, Bytes::from(object));
        Ok(())
    }

    async fn abort(&self, upload: MultipartUpload) -> Result<(), MultipartError> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .uploads
            .remove(&upload.upload_id);
        Ok(())
    }

    async fn abort_incomplete(&self, key: &str) -> Result<(), MultipartError> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .uploads
            .retain(|_, upload| upload.key != key);
        Ok(())
    }

    async fn read(&self, key: &str) -> Result<RemoteReader, MultipartError> {
        let bytes = self
            .state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .get(key)
            .cloned()
            .ok_or_else(|| MultipartError::new("object does not exist"))?;
        let (mut writer, reader) = tokio::io::duplex(bytes.len().max(1));
        tokio::spawn(async move {
            let _ = writer.write_all(&bytes).await;
        });
        Ok(Box::pin(reader))
    }

    async fn delete(&self, key: &str) -> Result<(), MultipartError> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .remove(key);
        Ok(())
    }
}
