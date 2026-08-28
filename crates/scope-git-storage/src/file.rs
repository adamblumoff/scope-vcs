use crate::{MultipartError, MultipartStore, MultipartUpload, RemoteReader, UploadedPart};
use async_trait::async_trait;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

#[derive(Clone, Debug)]
pub struct FileMultipartStore {
    root: PathBuf,
}

impl FileMultipartStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, MultipartError> {
        let root = root.into();
        if root.as_os_str().is_empty() {
            return Err(MultipartError::new("filesystem multipart root is required"));
        }
        Ok(Self { root })
    }

    fn uploads_root(&self) -> PathBuf {
        self.root.join("multipart")
    }

    fn upload_path(&self, upload_id: &str) -> Result<PathBuf, MultipartError> {
        validate_upload_id(upload_id)?;
        Ok(self.uploads_root().join(upload_id))
    }

    fn object_path(&self, key: &str) -> Result<PathBuf, MultipartError> {
        let components = validate_key(key)?;
        let mut path = self.root.join("objects");
        for component in components {
            path.push(component);
        }
        Ok(path)
    }

    async fn verify_upload(&self, upload: &MultipartUpload) -> Result<PathBuf, MultipartError> {
        validate_key(&upload.key)?;
        let directory = self.upload_path(&upload.upload_id)?;
        let recorded_key = fs::read_to_string(directory.join("key"))
            .await
            .map_err(MultipartError::from)?;
        if recorded_key != upload.key {
            return Err(MultipartError::new(
                "filesystem multipart upload key does not match",
            ));
        }
        Ok(directory)
    }
}

#[async_trait]
impl MultipartStore for FileMultipartStore {
    async fn begin(&self, key: &str) -> Result<MultipartUpload, MultipartError> {
        validate_key(key)?;
        fs::create_dir_all(self.uploads_root())
            .await
            .map_err(MultipartError::from)?;
        let upload_id = random_upload_id()?;
        let directory = self.upload_path(&upload_id)?;
        fs::create_dir(&directory)
            .await
            .map_err(MultipartError::from)?;
        let mut metadata = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join("key"))
            .await
            .map_err(MultipartError::from)?;
        metadata
            .write_all(key.as_bytes())
            .await
            .map_err(MultipartError::from)?;
        metadata.sync_all().await.map_err(MultipartError::from)?;
        sync_directory(directory.clone()).await?;
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
        if part_number <= 0 {
            return Err(MultipartError::new(
                "multipart part number must be positive",
            ));
        }
        let directory = self.verify_upload(upload).await?;
        let part_name = format!("{part_number:08}.part");
        let temp_path = directory.join(format!("{part_name}.tmp"));
        let final_path = directory.join(part_name);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .map_err(MultipartError::from)?;
        let result = async {
            file.write_all(&bytes).await.map_err(MultipartError::from)?;
            file.sync_all().await.map_err(MultipartError::from)?;
            drop(file);
            if fs::try_exists(&final_path)
                .await
                .map_err(MultipartError::from)?
            {
                return Err(MultipartError::new(
                    "filesystem multipart part already exists",
                ));
            }
            fs::rename(&temp_path, &final_path)
                .await
                .map_err(MultipartError::from)?;
            sync_directory(directory).await
        }
        .await;
        if let Err(error) = result {
            let _ = fs::remove_file(&temp_path).await;
            return Err(error);
        }
        Ok(UploadedPart {
            part_number,
            etag: hex::encode(Sha256::digest(&bytes)),
        })
    }

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), MultipartError> {
        if parts.is_empty() {
            return Err(MultipartError::new(
                "filesystem multipart upload has no parts",
            ));
        }
        let upload_directory = self.verify_upload(&upload).await?;
        let final_path = self.object_path(&upload.key)?;
        let parent = final_path
            .parent()
            .ok_or_else(|| MultipartError::new("filesystem object path has no parent"))?;
        fs::create_dir_all(parent)
            .await
            .map_err(MultipartError::from)?;
        if fs::try_exists(&final_path)
            .await
            .map_err(MultipartError::from)?
        {
            return Err(MultipartError::new(
                "filesystem multipart object already exists",
            ));
        }
        let file_name = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| MultipartError::new("filesystem object name is invalid"))?;
        let temp_path = parent.join(format!(".{file_name}.{}.tmp", upload.upload_id));
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .map_err(MultipartError::from)?;
        let result = async {
            for (index, part) in parts.iter().enumerate() {
                let expected = i32::try_from(index + 1)
                    .map_err(|_| MultipartError::new("multipart part count exceeds i32"))?;
                if part.part_number != expected {
                    return Err(MultipartError::new(
                        "filesystem multipart parts are not contiguous",
                    ));
                }
                let path = upload_directory.join(format!("{:08}.part", part.part_number));
                let mut input = File::open(path).await.map_err(MultipartError::from)?;
                tokio::io::copy(&mut input, &mut output)
                    .await
                    .map_err(MultipartError::from)?;
            }
            output.sync_all().await.map_err(MultipartError::from)?;
            drop(output);
            fs::rename(&temp_path, &final_path)
                .await
                .map_err(MultipartError::from)?;
            sync_directory(parent.to_path_buf()).await?;
            let _ = fs::remove_dir_all(&upload_directory).await;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_file(&temp_path).await;
        }
        result
    }

    async fn abort(&self, upload: MultipartUpload) -> Result<(), MultipartError> {
        let directory = self.verify_upload(&upload).await?;
        fs::remove_dir_all(directory)
            .await
            .map_err(MultipartError::from)
    }

    async fn abort_incomplete(&self, key: &str) -> Result<(), MultipartError> {
        validate_key(key)?;
        let uploads_root = self.uploads_root();
        let mut entries = match fs::read_dir(&uploads_root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(MultipartError::from(error)),
        };
        while let Some(entry) = entries.next_entry().await.map_err(MultipartError::from)? {
            let Some(upload_id) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if validate_upload_id(&upload_id).is_err() {
                continue;
            }
            let directory = self.upload_path(&upload_id)?;
            let recorded_key = match fs::read_to_string(directory.join("key")).await {
                Ok(recorded_key) => recorded_key,
                Err(_) => continue,
            };
            if recorded_key == key {
                fs::remove_dir_all(directory)
                    .await
                    .map_err(MultipartError::from)?;
            }
        }
        Ok(())
    }

    async fn read(&self, key: &str) -> Result<RemoteReader, MultipartError> {
        let path = self.object_path(key)?;
        let file = File::open(path).await.map_err(MultipartError::from)?;
        Ok(Box::pin(file))
    }

    async fn delete(&self, key: &str) -> Result<(), MultipartError> {
        let path = self.object_path(key)?;
        match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(MultipartError::from(error)),
        }
    }
}

fn validate_key(key: &str) -> Result<Vec<&str>, MultipartError> {
    if key.is_empty() || key.starts_with('/') || key.ends_with('/') || key.contains('\\') {
        return Err(MultipartError::new("filesystem object key is invalid"));
    }
    let components = key.split('/').collect::<Vec<_>>();
    if components.iter().any(|component| {
        component.is_empty()
            || *component == "."
            || *component == ".."
            || !component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    }) {
        return Err(MultipartError::new("filesystem object key is invalid"));
    }
    Ok(components)
}

fn validate_upload_id(upload_id: &str) -> Result<(), MultipartError> {
    if upload_id.len() != 32
        || !upload_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(MultipartError::new(
            "filesystem multipart upload id is invalid",
        ));
    }
    Ok(())
}

fn random_upload_id() -> Result<String, MultipartError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| MultipartError::new(format!("creating multipart upload id: {error}")))?;
    Ok(hex::encode(bytes))
}

async fn sync_directory(directory: PathBuf) -> Result<(), MultipartError> {
    tokio::task::spawn_blocking(move || std::fs::File::open(directory)?.sync_all())
        .await
        .map_err(|error| MultipartError::new(error.to_string()))?
        .map_err(MultipartError::from)
}
