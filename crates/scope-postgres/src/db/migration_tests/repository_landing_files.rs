use super::*;
use crate::db::landing_files::{apply_repository_landing_file_mutation, repository_landing_file};
use scope_domain::{
    content::DEFAULT_GIT_FILE_MODE,
    landing_file::{RepositoryLandingFile, RepositoryLandingFileMutation},
};
use sea_orm::ConnectionTrait;
use sha2::{Digest as _, Sha256};

async fn insert_repository(db: &DatabaseConnection) {
    db.execute_unprepared(
        "
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('landing-owner', 'landing-owner', 'landing@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'landing-owner/repo', 'landing-owner', 'repo', 'landing-owner', 'Ready',
            1, '{}'::jsonb, '{}'::jsonb, 'repoi_landing_owner_repo'
        );
        ",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn repository_landing_file_schema_enforces_identity_bounds_and_cascade() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    insert_repository(db.as_ref()).await;
    db.execute_unprepared(
        "
        INSERT INTO scope_repository_landing_files (
            repo_id, path, oid, sha256, size_bytes, git_file_mode, content_bytes
        ) VALUES (
            'landing-owner/repo', '/README.html', 'abc123', repeat('a', 64),
            1, '100644', decode('61', 'hex')
        );
        ",
    )
    .await
    .unwrap();

    assert!(
        db.execute_unprepared(
            "
            UPDATE scope_repository_landing_files
            SET path = '/readme.html'
            WHERE repo_id = 'landing-owner/repo'
            ",
        )
        .await
        .is_err()
    );
    assert!(
        db.execute_unprepared(
            "
            UPDATE scope_repository_landing_files
            SET size_bytes = 1048577,
                content_bytes = repeat('a', 1048577)::bytea
            WHERE repo_id = 'landing-owner/repo'
            ",
        )
        .await
        .is_err()
    );
    assert!(
        db.execute_unprepared(
            "
            UPDATE scope_repository_landing_files
            SET size_bytes = 2
            WHERE repo_id = 'landing-owner/repo'
            ",
        )
        .await
        .is_err()
    );

    db.execute_unprepared("DELETE FROM scope_repositories WHERE id = 'landing-owner/repo'")
        .await
        .unwrap();
    let remaining = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_repository_landing_files".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn repository_landing_file_mutations_create_replace_preserve_and_delete() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    insert_repository(db.as_ref()).await;

    let first_bytes = b"<h1>first</h1>".to_vec();
    let first = RepositoryLandingFile {
        oid: "first-oid".to_string(),
        sha256: hex::encode(Sha256::digest(&first_bytes)),
        size_bytes: first_bytes.len() as u64,
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        content_bytes: first_bytes,
    };
    apply_repository_landing_file_mutation(
        db.as_ref(),
        "landing-owner/repo",
        RepositoryLandingFileMutation::Upsert(first.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        repository_landing_file(db.as_ref(), "landing-owner/repo")
            .await
            .unwrap(),
        Some(first)
    );
    let before_unchanged = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT xmin::text AS xmin FROM scope_repository_landing_files".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "xmin")
        .unwrap();

    apply_repository_landing_file_mutation(
        db.as_ref(),
        "landing-owner/repo",
        RepositoryLandingFileMutation::Unchanged,
    )
    .await
    .unwrap();
    let after_unchanged = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT xmin::text AS xmin FROM scope_repository_landing_files".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "xmin")
        .unwrap();
    assert_eq!(after_unchanged, before_unchanged);

    let replacement_bytes = b"<h1>replacement</h1>".to_vec();
    let replacement = RepositoryLandingFile {
        oid: "replacement-oid".to_string(),
        sha256: hex::encode(Sha256::digest(&replacement_bytes)),
        size_bytes: replacement_bytes.len() as u64,
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        content_bytes: replacement_bytes,
    };
    apply_repository_landing_file_mutation(
        db.as_ref(),
        "landing-owner/repo",
        RepositoryLandingFileMutation::Upsert(replacement.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        repository_landing_file(db.as_ref(), "landing-owner/repo")
            .await
            .unwrap(),
        Some(replacement)
    );

    apply_repository_landing_file_mutation(
        db.as_ref(),
        "landing-owner/repo",
        RepositoryLandingFileMutation::Delete,
    )
    .await
    .unwrap();
    assert_eq!(
        repository_landing_file(db.as_ref(), "landing-owner/repo")
            .await
            .unwrap(),
        None
    );
}
