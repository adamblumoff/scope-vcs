use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    policy::ScopePath,
    projection::{LogicalCommitOrigin, NativePublicCommit},
    repository::{
        RepoLifecycleState,
        collaboration::{RepositoryInvite, RepositoryInviteState, RepositoryMemberPermissions},
    },
    runs::{
        source::RunSource,
        step::AttemptTerminalReason,
        workflow::definition::{
            CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep,
            WorkflowTriggers,
        },
        workflow::identity::{WorkflowIdentity, WorkflowPath},
        workflow::revision::WorkflowRevision,
    },
};
use serde_json::json;
use std::collections::BTreeMap;

const SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GIT_OID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn source_blob(content_ref: ContentRef) -> SourceBlob {
    SourceBlob {
        content_ref,
        sha256: SHA256.to_string(),
        git_oid: GIT_OID.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 42,
    }
}

#[test]
fn source_blob_persisted_json_shape_is_stable() {
    let blob = source_blob(ContentRef::blob_sha256(SHA256));

    assert_eq!(
        serde_json::to_value(blob).unwrap(),
        json!({
            "content_ref": { "BlobSha256": SHA256 },
            "sha256": SHA256,
            "git_oid": GIT_OID,
            "git_file_mode": "100644",
            "size_bytes": 42,
        })
    );
}

#[test]
fn logical_commit_origin_persisted_json_shape_is_stable() {
    let origin = LogicalCommitOrigin::PublicRequestMerge {
        request_id: "request-7".to_string(),
        public_base_oid: "1111111111111111111111111111111111111111".to_string(),
        public_parent_oids: vec!["2222222222222222222222222222222222222222".to_string()],
        request_head_oid: "3333333333333333333333333333333333333333".to_string(),
        commits: vec![NativePublicCommit {
            oid: "4444444444444444444444444444444444444444".to_string(),
            parent_oids: vec!["2222222222222222222222222222222222222222".to_string()],
            tree_oid: "5555555555555555555555555555555555555555".to_string(),
            changed_paths: vec![ScopePath::parse("/src/lib.rs").unwrap()],
        }],
        preserve_public_commits: true,
    };

    assert_eq!(
        serde_json::to_value(origin).unwrap(),
        json!({
            "PublicRequestMerge": {
                "request_id": "request-7",
                "public_base_oid": "1111111111111111111111111111111111111111",
                "public_parent_oids": ["2222222222222222222222222222222222222222"],
                "request_head_oid": "3333333333333333333333333333333333333333",
                "commits": [{
                    "oid": "4444444444444444444444444444444444444444",
                    "parent_oids": ["2222222222222222222222222222222222222222"],
                    "tree_oid": "5555555555555555555555555555555555555555",
                    "changed_paths": ["/src/lib.rs"],
                }],
                "preserve_public_commits": true,
            }
        })
    );
}

#[test]
fn repository_lifecycle_invite_and_permissions_persisted_json_shapes_are_stable() {
    let permissions = RepositoryMemberPermissions {
        can_push: true,
        can_change_file_visibility: false,
        can_apply_changes: true,
    };
    let invite = RepositoryInvite {
        id: "invite-1".to_string(),
        repo_id: "owner/repo".to_string(),
        invited_email: "Maintainer@example.com".to_string(),
        invited_email_normalized: "maintainer@example.com".to_string(),
        permissions,
        invited_by_user_id: "owner-user".to_string(),
        state: RepositoryInviteState::Pending,
        token_hash: "token-hash".to_string(),
        created_at_unix: 100,
        updated_at_unix: 101,
        expires_at_unix: 200,
        accepted_by_user_id: None,
        accepted_at_unix: None,
        revoked_at_unix: None,
    };

    assert_eq!(
        serde_json::to_value(RepoLifecycleState::AwaitingFirstPush).unwrap(),
        json!("AwaitingFirstPush")
    );
    assert_eq!(
        serde_json::to_value(permissions).unwrap(),
        json!({
            "can_push": true,
            "can_change_file_visibility": false,
            "can_apply_changes": true,
        })
    );
    assert_eq!(
        serde_json::to_value(invite).unwrap(),
        json!({
            "id": "invite-1",
            "repo_id": "owner/repo",
            "invited_email": "Maintainer@example.com",
            "invited_email_normalized": "maintainer@example.com",
            "permissions": {
                "can_push": true,
                "can_change_file_visibility": false,
                "can_apply_changes": true,
            },
            "invited_by_user_id": "owner-user",
            "state": "Pending",
            "token_hash": "token-hash",
            "created_at_unix": 100,
            "updated_at_unix": 101,
            "expires_at_unix": 200,
            "accepted_by_user_id": null,
            "accepted_at_unix": null,
            "revoked_at_unix": null,
        })
    );
}

#[test]
fn run_source_and_terminal_reason_persisted_json_shapes_are_stable() {
    let source = RunSource::EphemeralGitBundle {
        object: source_blob(ContentRef::git_bundle_sha256(SHA256)),
    };
    let reason = AttemptTerminalReason::RuntimeSetupFailed {
        exit_code: 127,
        message: "runtime unavailable".to_string(),
    };

    assert_eq!(
        serde_json::to_value(source).unwrap(),
        json!({
            "kind": "ephemeral-git-bundle",
            "object": {
                "content_ref": { "GitBundleSha256": SHA256 },
                "sha256": SHA256,
                "git_oid": GIT_OID,
                "git_file_mode": "100644",
                "size_bytes": 42,
            }
        })
    );
    assert_eq!(
        serde_json::to_value(reason).unwrap(),
        json!({
            "kind": "runtime-setup-failed",
            "exit_code": 127,
            "message": "runtime unavailable",
        })
    );
}

#[test]
fn compiled_workflow_persisted_json_and_revision_digest_are_stable() {
    let definition = CompiledWorkflow::new(
        "Checks",
        WorkflowTriggers::new(true, true).unwrap(),
        vec![
            WorkflowJob::new(
                WorkflowJobId::parse("build").unwrap(),
                Vec::new(),
                ContainerSpec::new(format!("rust@sha256:{SHA256}")).unwrap(),
                600,
                Vec::new(),
                BTreeMap::from([("RUST_BACKTRACE".to_string(), "1".to_string())]),
                vec![WorkflowStep::new("Build", "cargo build").unwrap()],
            )
            .unwrap(),
        ],
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(&definition).unwrap(),
        json!({
            "name": "Checks",
            "triggers": {
                "manual": true,
                "push_main": true,
            },
            "jobs": [{
                "id": "build",
                "needs": [],
                "container": {
                    "image": format!("rust@sha256:{SHA256}"),
                },
                "timeout_seconds": 600,
                "caches": [],
                "environment": {
                    "RUST_BACKTRACE": "1",
                },
                "steps": [{
                    "name": "Build",
                    "run": "cargo build",
                }],
            }],
        })
    );

    let revision = WorkflowRevision::new(
        WorkflowIdentity::new(
            "owner/repo",
            WorkflowPath::parse("/.scope/runs/checks.yml").unwrap(),
        )
        .unwrap(),
        definition,
    )
    .unwrap();
    assert_eq!(
        revision.digest(),
        "0740cd8887d731cfb569b266b9228cba9edec98aff5780a0f9ee006815f62699"
    );
}
