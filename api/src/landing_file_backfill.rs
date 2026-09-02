use crate::{git::content::source_content_bytes, state::AppState};
use scope_domain::landing_file::RepositoryLandingFile;

impl AppState {
    pub async fn backfill_repository_landing_files(&self) -> anyhow::Result<usize> {
        let candidates = self
            .metadata
            .repositories()
            .repository_landing_file_backfill_candidates()
            .await?;
        let mut stored = 0;
        for candidate in candidates {
            let git_source = candidate.git_head.as_ref().map(|head| {
                (
                    candidate.incarnation.clone(),
                    head,
                    candidate.git_pack_spans.as_slice(),
                )
            });
            let bytes = source_content_bytes(self, &candidate.blob, git_source)
                .await
                .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
            let landing_file = RepositoryLandingFile::from_source_blob(&candidate.blob, bytes)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            self.metadata
                .repositories()
                .store_backfilled_repository_landing_file(&candidate.repo_id, landing_file)
                .await?;
            let _ = self
                .repository_engine
                .delete_repository_cache(&candidate.incarnation)
                .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
            stored += 1;
        }
        let remaining = self
            .metadata
            .repositories()
            .repository_landing_file_backfill_candidates()
            .await?;
        if !remaining.is_empty() {
            anyhow::bail!(
                "{} repository landing files remain without snapshots",
                remaining.len()
            );
        }
        Ok(stored)
    }
}
