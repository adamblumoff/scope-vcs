use crate::error::MultipartError;
use async_trait::async_trait;
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, SharedCredentialsProvider, retry::RetryConfig},
    error::SdkError,
    operation::get_object::GetObjectError,
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart},
};
use bytes::Bytes;
use std::{error::Error, future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::io::AsyncRead;

const REMOTE_READ_ATTEMPTS: usize = 3;

type GetObjectSdkError = SdkError<GetObjectError>;

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

fn s3_request_error(
    operation: &str,
    key: &str,
    error: impl Error + Send + Sync + 'static,
) -> MultipartError {
    MultipartError::with_source(format!("S3 {operation} failed for object {key}"), error)
}

fn is_retryable_get_error(error: &GetObjectSdkError) -> bool {
    match error {
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => {
            true
        }
        SdkError::ServiceError(error) => {
            matches!(error.raw().status().as_u16(), 429 | 500 | 502 | 503 | 504)
        }
        SdkError::ConstructionFailure(_) => false,
        _ => false,
    }
}

async fn retry_remote_read<T, E, Operation, OperationFuture, Retryable, Sleep, SleepFuture>(
    mut operation: Operation,
    retryable: Retryable,
    mut sleep: Sleep,
) -> Result<T, E>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T, E>>,
    Retryable: Fn(&E) -> bool,
    Sleep: FnMut(Duration) -> SleepFuture,
    SleepFuture: Future<Output = ()>,
{
    let mut attempt = 1;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if attempt < REMOTE_READ_ATTEMPTS && retryable(&error) => {
                sleep(remote_read_delay(attempt)).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn remote_read_delay(failed_attempt: usize) -> Duration {
    let base_millis = 25_u64 << failed_attempt.saturating_sub(1);
    let mut random = [0_u8; 1];
    let jitter = if getrandom::fill(&mut random).is_ok() {
        u64::from(random[0]) * base_millis / 510
    } else {
        0
    };
    Duration::from_millis(base_millis + jitter)
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
            .map_err(|error| s3_request_error("create multipart upload", key, error))?;
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
            .map_err(|error| s3_request_error("upload part", &upload.key, error))?;
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
            .key(&upload.key)
            .upload_id(upload.upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|error| s3_request_error("complete multipart upload", &upload.key, error))?;
        Ok(())
    }

    async fn abort(&self, upload: MultipartUpload) -> Result<(), MultipartError> {
        self.client
            .abort_multipart_upload()
            .bucket(self.bucket.as_ref())
            .key(&upload.key)
            .upload_id(upload.upload_id)
            .send()
            .await
            .map_err(|error| s3_request_error("abort multipart upload", &upload.key, error))?;
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
                .map_err(|error| s3_request_error("list multipart uploads", key, error))?;
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
                        .map_err(|error| {
                            s3_request_error("abort multipart upload", upload_key, error)
                        })?;
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
        let response = retry_remote_read(
            || {
                self.client
                    .get_object()
                    .bucket(self.bucket.as_ref())
                    .key(key)
                    .customize()
                    .config_override(
                        aws_sdk_s3::config::Builder::new()
                            .retry_config(RetryConfig::standard().with_max_attempts(1)),
                    )
                    .send()
            },
            is_retryable_get_error,
            tokio::time::sleep,
        )
        .await
        .map_err(|error| s3_request_error("get object", key, error))?;
        Ok(Box::pin(response.body.into_async_read()))
    }

    async fn delete(&self, key: &str) -> Result<(), MultipartError> {
        self.client
            .delete_object()
            .bucket(self.bucket.as_ref())
            .key(key)
            .send()
            .await
            .map_err(|error| s3_request_error("delete object", key, error))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_s3::{config::http::HttpResponse, error::ErrorMetadata, primitives::SdkBody};
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, thiserror::Error)]
    #[error("test read failed")]
    struct TestReadError {
        retryable: bool,
    }

    fn service_error(code: &str, status: u16) -> GetObjectSdkError {
        let error = GetObjectError::generic(ErrorMetadata::builder().code(code).build());
        let response = HttpResponse::new(status.try_into().unwrap(), SdkBody::empty());
        SdkError::service_error(error, response)
    }

    #[test]
    fn get_object_retries_only_temporary_failures() {
        let timeout = GetObjectSdkError::timeout_error(io::Error::new(
            io::ErrorKind::TimedOut,
            "request timed out",
        ));
        let construction = GetObjectSdkError::construction_failure(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid request",
        ));

        assert!(is_retryable_get_error(&timeout));
        assert!(is_retryable_get_error(&service_error(
            "ServiceUnavailable",
            503,
        )));
        assert!(is_retryable_get_error(&service_error("SlowDown", 429)));
        assert!(is_retryable_get_error(&service_error("InternalError", 500)));
        assert!(!is_retryable_get_error(
            &service_error("AccessDenied", 403,)
        ));
        assert!(!is_retryable_get_error(&service_error("NoSuchKey", 404)));
        assert!(!is_retryable_get_error(&construction));
    }

    #[tokio::test]
    async fn remote_read_succeeds_after_retryable_failures() {
        let attempts = AtomicUsize::new(0);
        let sleeps = AtomicUsize::new(0);

        let result = retry_remote_read(
            || async {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(TestReadError { retryable: true })
                } else {
                    Ok("object")
                }
            },
            |error| error.retryable,
            |_| async {
                sleeps.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await;

        assert_eq!(result.unwrap(), "object");
        assert_eq!(attempts.load(Ordering::SeqCst), REMOTE_READ_ATTEMPTS);
        assert_eq!(sleeps.load(Ordering::SeqCst), REMOTE_READ_ATTEMPTS - 1);
    }

    #[tokio::test]
    async fn remote_read_stops_after_non_retryable_failure() {
        let attempts = AtomicUsize::new(0);
        let sleeps = AtomicUsize::new(0);

        let result = retry_remote_read(
            || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(TestReadError { retryable: false })
            },
            |error| error.retryable,
            |_| async {
                sleeps.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await;

        assert!(!result.unwrap_err().retryable);
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(sleeps.load(Ordering::SeqCst), 0);
    }
}
