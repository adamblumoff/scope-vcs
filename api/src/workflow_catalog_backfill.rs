use crate::{git::content::source_content_bytes, state::AppState};
use scope_domain::runs::{
    catalog::{
        MAX_REPOSITORY_WORKFLOW_FILES, MAX_WORKFLOW_DEFINITION_BYTES, RepositoryWorkflowCatalog,
        RepositoryWorkflowFile,
    },
    workflow::WorkflowPath,
};

impl AppState {
    pub async fn backfill_repository_workflow_catalogs(&self) -> anyhow::Result<usize> {
        let candidates = self
            .metadata
            .repositories()
            .repository_workflow_catalog_backfill_candidates()
            .await?;
        let mut stored = 0;
        for candidate in candidates {
            let catalog = if candidate.workflow_blobs.len() > MAX_REPOSITORY_WORKFLOW_FILES {
                RepositoryWorkflowCatalog::rejected(
                    &candidate.repo_id,
                    &candidate.git_head.head_oid,
                    candidate.source_change_version,
                    format!(
                        "repository contains more than {MAX_REPOSITORY_WORKFLOW_FILES} workflow definitions"
                    ),
                )?
            } else {
                let mut files = Vec::with_capacity(candidate.workflow_blobs.len());
                let mut rejection = None;
                for (path, blob) in &candidate.workflow_blobs {
                    if WorkflowPath::parse(path).is_err() {
                        rejection = Some(format!("invalid workflow path {path}"));
                        break;
                    }
                    if blob.size_bytes > MAX_WORKFLOW_DEFINITION_BYTES as u64 {
                        rejection = Some(format!(
                            "workflow {path} exceeds {MAX_WORKFLOW_DEFINITION_BYTES} bytes"
                        ));
                        break;
                    }
                    let bytes = source_content_bytes(
                        self,
                        blob,
                        Some((
                            candidate.repo_id.as_str(),
                            &candidate.git_head,
                            candidate.git_pack_spans.as_slice(),
                        )),
                    )
                    .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
                    files.push(RepositoryWorkflowFile::from_source_blob(path, blob, bytes)?);
                }
                match rejection {
                    Some(error) => RepositoryWorkflowCatalog::rejected(
                        &candidate.repo_id,
                        &candidate.git_head.head_oid,
                        candidate.source_change_version,
                        error,
                    )?,
                    None => RepositoryWorkflowCatalog::captured(
                        &candidate.repo_id,
                        &candidate.git_head.head_oid,
                        candidate.source_change_version,
                        files,
                    )?,
                }
            };
            if self
                .metadata
                .repositories()
                .store_backfilled_repository_workflow_catalog(&catalog)
                .await?
            {
                stored += 1;
            }
        }
        let remaining = self
            .metadata
            .repositories()
            .repository_workflow_catalog_backfill_candidates()
            .await?;
        if !remaining.is_empty() {
            anyhow::bail!(
                "{} repositories remain without workflow catalogs",
                remaining.len()
            );
        }
        Ok(stored)
    }
}
