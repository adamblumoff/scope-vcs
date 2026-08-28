use super::{GitSegmentStore, GitStorageError, MultipartError, StagedGitSegment, valid_segment_id};
use std::path::PathBuf;
use tokio::fs;

impl GitSegmentStore {
    pub async fn delete_remote(&self, object_key: &str) -> Result<(), GitStorageError> {
        self.backend.delete(object_key).await.map_err(Into::into)
    }

    pub async fn cleanup_remote(&self, object_key: &str) -> Result<(), GitStorageError> {
        let abort = self.backend.abort_incomplete(object_key).await;
        let delete = self.backend.delete(object_key).await;
        match (abort, delete) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(abort), Ok(())) => Err(GitStorageError::Multipart(abort)),
            (Ok(()), Err(delete)) => Err(GitStorageError::Multipart(delete)),
            (Err(abort), Err(delete)) => {
                Err(GitStorageError::Multipart(MultipartError::new(format!(
                    "aborting incomplete uploads failed: {abort}; deleting object failed: {delete}"
                ))))
            }
        }
    }

    pub async fn delete_local(&self, staged: &StagedGitSegment) -> Result<(), GitStorageError> {
        match fs::remove_file(staged.local_pack_path()).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(GitStorageError::Local(error)),
        }
    }

    pub async fn cleanup_local(
        &self,
        repository_id: &str,
        segment_id: &str,
    ) -> Result<(), GitStorageError> {
        if repository_id.is_empty() || !valid_segment_id(segment_id) {
            return Err(GitStorageError::InvalidConfiguration(
                "repository id or segment id is invalid".into(),
            ));
        }
        let directory = self.local_directory(repository_id);
        for name in [
            format!("{segment_id}.pack.tmp"),
            format!("{segment_id}.pack"),
        ] {
            match fs::remove_file(directory.join(name)).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(GitStorageError::Local(error)),
            }
        }
        if fs::try_exists(&directory)
            .await
            .map_err(GitStorageError::Local)?
        {
            sync_directory(directory).await?;
        }
        Ok(())
    }
}

pub(super) async fn sync_directory(directory: PathBuf) -> Result<(), GitStorageError> {
    tokio::task::spawn_blocking(move || std::fs::File::open(directory)?.sync_all())
        .await
        .map_err(|error| GitStorageError::Task(error.to_string()))?
        .map_err(GitStorageError::Local)
}
