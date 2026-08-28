use crate::error::MultipartError;
use async_trait::async_trait;
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, SharedCredentialsProvider},
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart},
};
use bytes::Bytes;
use std::{pin::Pin, sync::Arc};
use tokio::io::AsyncRead;

pub type RemoteReader = Pin<Box<dyn AsyncRead + Send>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultipartUpload {
    pub key: String,
    pub upload_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadedPart {
    pub part_number: i32,
    pub etag: String,
}

#[async_trait]
pub trait MultipartStore: Send + Sync + 'static {
    fn minimum_part_bytes(&self) -> usize {
        1
    }

    async fn begin(&self, key: &str) -> Result<MultipartUpload, MultipartError>;

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        bytes: Bytes,
    ) -> Result<UploadedPart, MultipartError>;

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), MultipartError>;

    async fn abort(&self, upload: MultipartUpload) -> Result<(), MultipartError>;

    async fn abort_incomplete(&self, key: &str) -> Result<(), MultipartError>;

    async fn read(&self, key: &str) -> Result<RemoteReader, MultipartError>;

    async fn delete(&self, key: &str) -> Result<(), MultipartError>;
}

#[derive(Clone)]
pub struct S3MultipartSettings {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
}

#[derive(Clone)]
pub struct S3MultipartStore {
    client: Client,
    bucket: Arc<str>,
}

impl S3MultipartStore {
    pub fn new(settings: S3MultipartSettings) -> Result<Self, MultipartError> {
        if settings.endpoint.trim().is_empty()
            || settings.bucket.trim().is_empty()
            || settings.region.trim().is_empty()
            || settings.access_key_id.trim().is_empty()
            || settings.secret_access_key.is_empty()
        {
            return Err(MultipartError::new(
                "S3 endpoint, bucket, region, and credentials are required",
            ));
        }
        let credentials = Credentials::new(
            settings.access_key_id,
            settings.secret_access_key,
            None,
            None,
            "scope-git-storage",
        );
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(settings.endpoint.trim_end_matches('/'))
            .region(Region::new(settings.region))
            .credentials_provider(SharedCredentialsProvider::new(credentials))
            .force_path_style(settings.force_path_style)
            .build();
        Ok(Self {
            client: Client::from_conf(config),
            bucket: Arc::from(settings.bucket),
        })
    }
}

#[async_trait]
impl MultipartStore for S3MultipartStore {
    fn minimum_part_bytes(&self) -> usize {
        5 * 1024 * 1024
    }

    async fn begin(&self, key: &str) -> Result<MultipartUpload, MultipartError> {
        let response = self
            .client
            .create_multipart_upload()
            .bucket(self.bucket.as_ref())
            .key(key)
            .send()
            .await
            .map_err(|error| MultipartError::new(error.to_string()))?;
        let upload_id = response
            .upload_id()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| MultipartError::new("S3 did not return a multipart upload id"))?;
        Ok(MultipartUpload {
            key: key.to_string(),
            upload_id: upload_id.to_string(),
        })
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        bytes: Bytes,
    ) -> Result<UploadedPart, MultipartError> {
        let response = self
            .client
            .upload_part()
            .bucket(self.bucket.as_ref())
            .key(&upload.key)
            .upload_id(&upload.upload_id)
            .part_number(part_number)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|error| MultipartError::new(error.to_string()))?;
        let etag = response
            .e_tag()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| MultipartError::new("S3 did not return an ETag for a multipart part"))?;
        Ok(UploadedPart {
            part_number,
            etag: etag.to_string(),
        })
    }

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), MultipartError> {
        let parts = parts
            .into_iter()
            .map(|part| {
                CompletedPart::builder()
                    .part_number(part.part_number)
                    .e_tag(part.etag)
                    .build()
            })
            .collect::<Vec<_>>();
        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();
        self.client
            .complete_multipart_upload()
            .bucket(self.bucket.as_ref())
            .key(upload.key)
            .upload_id(upload.upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|error| MultipartError::new(error.to_string()))?;
        Ok(())
    }

    async fn abort(&self, upload: MultipartUpload) -> Result<(), MultipartError> {
        self.client
            .abort_multipart_upload()
            .bucket(self.bucket.as_ref())
            .key(upload.key)
            .upload_id(upload.upload_id)
            .send()
            .await
            .map_err(|error| MultipartError::new(error.to_string()))?;
        Ok(())
    }

    async fn abort_incomplete(&self, key: &str) -> Result<(), MultipartError> {
        let mut key_marker = None;
        let mut upload_id_marker = None;
        loop {
            let response = self
                .client
                .list_multipart_uploads()
                .bucket(self.bucket.as_ref())
                .prefix(key)
                .set_key_marker(key_marker.clone())
                .set_upload_id_marker(upload_id_marker.clone())
                .send()
                .await
                .map_err(|error| MultipartError::new(error.to_string()))?;
            for upload in response.uploads() {
                let Some(upload_key) = upload.key() else {
                    continue;
                };
                let Some(upload_id) = upload.upload_id() else {
                    continue;
                };
                if upload_key == key {
                    self.client
                        .abort_multipart_upload()
                        .bucket(self.bucket.as_ref())
                        .key(upload_key)
                        .upload_id(upload_id)
                        .send()
                        .await
                        .map_err(|error| MultipartError::new(error.to_string()))?;
                }
            }
            if response.is_truncated() != Some(true) {
                return Ok(());
            }
            key_marker = response.next_key_marker().map(ToOwned::to_owned);
            upload_id_marker = response.next_upload_id_marker().map(ToOwned::to_owned);
            if key_marker.is_none() {
                return Err(MultipartError::new(
                    "S3 multipart listing was truncated without a next key marker",
                ));
            }
        }
    }

    async fn read(&self, key: &str) -> Result<RemoteReader, MultipartError> {
        let response = self
            .client
            .get_object()
            .bucket(self.bucket.as_ref())
            .key(key)
            .send()
            .await
            .map_err(|error| MultipartError::new(error.to_string()))?;
        Ok(Box::pin(response.body.into_async_read()))
    }

    async fn delete(&self, key: &str) -> Result<(), MultipartError> {
        self.client
            .delete_object()
            .bucket(self.bucket.as_ref())
            .key(key)
            .send()
            .await
            .map_err(|error| MultipartError::new(error.to_string()))?;
        Ok(())
    }
}
