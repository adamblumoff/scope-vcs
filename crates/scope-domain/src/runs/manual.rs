use super::{
    catalog::{RepositoryWorkflowCatalog, RepositoryWorkflowFile},
    run::Run,
    source::{RunSource, RunTrigger},
    workflow::{identity::WorkflowPath, revision::WorkflowRevision},
};
use crate::{
    error::DomainError,
    repository::access::{RepositoryAccess, RepositoryActor},
};

#[derive(Clone, Debug)]
pub struct ManualRunRequest {
    repository_id: String,
    user_id: String,
    request_id: String,
    git_oid: String,
    workflow_name: String,
}

impl ManualRunRequest {
    pub fn new(
        repository_id: String,
        user_id: String,
        request_id: String,
        git_oid: String,
        workflow_name: String,
    ) -> Result<Self, DomainError> {
        if repository_id.trim().is_empty() || user_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "manual run repository and user are required",
            ));
        }
        if request_id.len() != 32 || !request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::invalid_input(
                "request_id must be a 32-character hex string",
            ));
        }
        if git_oid.len() != 40 || !git_oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::invalid_input(
                "git_oid must be a 40-character hex string",
            ));
        }
        WorkflowPath::parse(format!("/.scope/runs/{workflow_name}.yml"))
            .map_err(DomainError::invalid_input)?;
        Ok(Self {
            repository_id,
            user_id,
            request_id,
            git_oid: git_oid.to_ascii_lowercase(),
            workflow_name,
        })
    }

    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    pub fn git_oid(&self) -> &str {
        &self.git_oid
    }
    pub fn workflow_name(&self) -> &str {
        &self.workflow_name
    }
    pub fn run_id(&self) -> String {
        format!("run_{}", self.request_id)
    }

    pub fn require_access(&self, access: RepositoryAccess) -> Result<(), DomainError> {
        if access.actor == RepositoryActor::Public {
            return Err(DomainError::forbidden("repo membership required"));
        }
        Ok(())
    }

    pub fn require_matching_run(&self, run: &Run) -> Result<(), DomainError> {
        if run.id == self.run_id()
            && run.idempotency_key == format!("manual:{}", self.request_id)
            && run.workflow.repository_id() == self.repository_id
            && run.workflow.path().name() == self.workflow_name
            && run.requested_by_user_id.as_deref() == Some(self.user_id.as_str())
            && run.trigger == RunTrigger::Manual
            && run.source.git_oid() == self.git_oid
        {
            Ok(())
        } else {
            Err(DomainError::conflict(
                "run request_id is already used by a different manual run",
            ))
        }
    }

    pub fn workflow_file<'a>(
        &self,
        catalog: &'a RepositoryWorkflowCatalog,
    ) -> Result<&'a RepositoryWorkflowFile, DomainError> {
        catalog
            .verify_source(
                &self.repository_id,
                &self.git_oid,
                catalog.source_change_version(),
            )
            .map_err(DomainError::invalid_input)?;
        if let Some(error) = catalog.configuration_error() {
            return Err(DomainError::invalid_input(error));
        }
        let mut matches = catalog
            .files()
            .unwrap_or_default()
            .iter()
            .filter(|file| file.path().name() == self.workflow_name);
        let file = matches.next().ok_or_else(|| {
            DomainError::not_found(format!(
                "workflow {:?} was not found at commit {}",
                self.workflow_name, self.git_oid
            ))
        })?;
        if matches.next().is_some() {
            return Err(DomainError::invalid_input(format!(
                "workflow {:?} is defined by both .yml and .yaml",
                self.workflow_name
            )));
        }
        Ok(file)
    }

    pub fn create_run(
        &self,
        revision: &WorkflowRevision,
        source: RunSource,
        now_unix: u64,
    ) -> Result<Run, DomainError> {
        if source.git_oid() != self.git_oid
            || revision.workflow().repository_id() != self.repository_id
            || revision.workflow().path().name() != self.workflow_name
        {
            return Err(DomainError::invalid_input(
                "manual run source and workflow do not match the request",
            ));
        }
        if let Some((repository_id, _, _)) = source.logical_git_head()
            && repository_id != self.repository_id
        {
            return Err(DomainError::invalid_input(
                "manual run source belongs to another repository",
            ));
        }
        let run = Run::new(
            self.run_id(),
            format!("manual:{}", self.request_id),
            revision.workflow().clone(),
            revision.digest(),
            RunTrigger::Manual,
            Some(self.user_id.clone()),
            source,
            now_unix,
        )?;
        run.validate_workflow_revision(revision)?;
        Ok(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
        content_ref::ContentRef,
        runs::workflow::identity::WorkflowIdentity,
    };

    fn request() -> ManualRunRequest {
        ManualRunRequest::new(
            "owner/repo".into(),
            "user-owner".into(),
            "1".repeat(32),
            "a".repeat(40),
            "checks".into(),
        )
        .unwrap()
    }

    #[test]
    fn replay_is_bound_to_repository_requester_workflow_and_commit() {
        let request = request();
        let source = RunSource::ephemeral_git_bundle(SourceBlob {
            content_ref: ContentRef::git_bundle_sha256("b".repeat(64)),
            sha256: "b".repeat(64),
            git_oid: "a".repeat(40),
            git_file_mode: DEFAULT_GIT_FILE_MODE.into(),
            size_bytes: 100,
        })
        .unwrap();
        let run = Run::new(
            request.run_id(),
            format!("manual:{}", "1".repeat(32)),
            WorkflowIdentity::new(
                "owner/repo",
                WorkflowPath::parse("/.scope/runs/checks.yml").unwrap(),
            )
            .unwrap(),
            "c".repeat(64),
            RunTrigger::Manual,
            Some("user-owner".into()),
            source,
            1,
        )
        .unwrap();
        request.require_matching_run(&run).unwrap();
        let mut wrong_user = run.clone();
        wrong_user.requested_by_user_id = Some("different-maintainer".into());
        assert!(request.require_matching_run(&wrong_user).is_err());
        let mut wrong_repo = run.clone();
        wrong_repo.workflow =
            WorkflowIdentity::new("owner/other", run.workflow.path().clone()).unwrap();
        assert!(request.require_matching_run(&wrong_repo).is_err());
        let mut wrong_workflow = run.clone();
        wrong_workflow.workflow = WorkflowIdentity::new(
            "owner/repo",
            WorkflowPath::parse("/.scope/runs/other.yml").unwrap(),
        )
        .unwrap();
        assert!(request.require_matching_run(&wrong_workflow).is_err());
        let mut wrong_head = run;
        if let RunSource::EphemeralGitBundle { object } = &mut wrong_head.source {
            object.git_oid = "d".repeat(40);
        }
        assert!(request.require_matching_run(&wrong_head).is_err());
    }

    #[test]
    fn known_workflow_requires_exact_catalog_and_one_definition() {
        let request = request();
        let file = RepositoryWorkflowFile::from_content(
            "/.scope/runs/checks.yml",
            DEFAULT_GIT_FILE_MODE,
            b"workflow".to_vec(),
        )
        .unwrap();
        let catalog = RepositoryWorkflowCatalog::captured(
            "owner/repo",
            "a".repeat(40),
            1,
            vec![file.clone()],
        )
        .unwrap();
        assert_eq!(request.workflow_file(&catalog).unwrap(), &file);
        let stale = RepositoryWorkflowCatalog::captured(
            "owner/repo",
            "d".repeat(40),
            1,
            vec![file.clone()],
        )
        .unwrap();
        assert!(request.workflow_file(&stale).is_err());
        let other_repo = RepositoryWorkflowCatalog::captured(
            "owner/other",
            "a".repeat(40),
            1,
            vec![file.clone()],
        )
        .unwrap();
        assert!(request.workflow_file(&other_repo).is_err());
        let other_extension = RepositoryWorkflowFile::from_content(
            "/.scope/runs/checks.yaml",
            DEFAULT_GIT_FILE_MODE,
            b"workflow".to_vec(),
        )
        .unwrap();
        let ambiguous = RepositoryWorkflowCatalog::captured(
            "owner/repo",
            "a".repeat(40),
            1,
            vec![file, other_extension],
        )
        .unwrap();
        assert!(
            request
                .workflow_file(&ambiguous)
                .unwrap_err()
                .message
                .contains("both .yml and .yaml")
        );
    }
}
