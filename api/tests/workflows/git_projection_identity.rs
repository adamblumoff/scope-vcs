use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permissioned_scope_sessions_share_raw_live_head() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("owner-upload-snapshot");
    fs::write(source.join("README.md"), "raw snapshot").unwrap();
    run_git(Some(&source), &["add", "-A"], "add readme").unwrap();
    commit_all(&source, "raw snapshot commit");
    let bare = clone_test_repo(&source, "owner-upload-snapshot-bare", true);
    let expected =
        git_stdout_text(&bare, &["rev-parse", DEFAULT_GIT_BRANCH], "snapshot head").unwrap();
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.publication_state = RepoPublicationState::Unpublished;
            repo.record.default_visibility = Visibility::Private;
            repo.policy = Policy::new(Visibility::Private);
        })
        .await
        .unwrap();
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Private)).await;

    let member_subject = "user_member";
    let member_id = crate::db::scope_user_id_for_auth_identity("clerk", member_subject);
    state
        .metadata
        .insert_user_for_tests(test_user(member_id.clone(), "member", "member@example.com"))
        .await
        .unwrap();
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.push(test_repository_member(
                TEST_REPO_ID,
                member_id,
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();

    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    let actors = [
        ("owner", bearer_header()),
        (
            "member",
            bearer_header_for(member_subject, "member@example.com"),
        ),
    ];
    let mut heads = Vec::new();
    for (actor, bearer) in actors {
        let clone = TempGitRepo(unique_test_path(&format!("{actor}-private-clone")));
        clone_with_bearer(&remote, &clone, &bearer, &format!("clone as {actor}"));
        heads.push(git_stdout_text(&clone, &["rev-parse", "HEAD"], "clone head").unwrap());
        assert!(!clone.join(".scope/repo.json").exists());
    }
    assert!(heads.iter().all(|head| head == &expected));
}
