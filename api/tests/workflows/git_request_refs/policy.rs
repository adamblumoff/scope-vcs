use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn advertisement_and_exact_fetch_follow_viewer_and_publication_policy() {
    let state = test_state_with_request().await;
    let (_author_checkout, permissioned_remote, _server, request_head) =
        request_checkout(&state, "request-advertisement-source").await;
    insert_public_contributor(&state).await;
    state
        .metadata
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            target_handle: "contributor".to_string(),
            now_unix: 3,
        })
        .await
        .unwrap();
    insert_member_user(&state).await;
    state
        .metadata
        .insert_user_for_tests(test_user(unrelated_user_id(), "unrelated", UNRELATED_EMAIL))
        .await
        .unwrap();
    let public_remote = permissioned_remote.replace("/permissioned/", "/public/");
    let author = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let invitee = bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL);
    let maintainer = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let unrelated = bearer_header_for(UNRELATED_SUBJECT, UNRELATED_EMAIL);

    assert!(advertises_request_ref(&permissioned_remote, Some(&author)));
    assert!(advertises_request_ref(&permissioned_remote, Some(&invitee)));
    assert!(!advertises_request_ref(
        &permissioned_remote,
        Some(&maintainer)
    ));
    assert!(!advertises_request_ref(
        &permissioned_remote,
        Some(&unrelated)
    ));
    assert!(!advertises_request_ref(&public_remote, None));
    assert!(fetch_exact_request_tip(
        &permissioned_remote,
        Some(&maintainer),
        &request_head,
        "maintainer-draft-exact-fetch",
    ));
    assert!(!fetch_exact_request_tip(
        &permissioned_remote,
        Some(&unrelated),
        &request_head,
        "unrelated-draft-exact-fetch",
    ));
    assert!(!fetch_exact_request_tip(
        &public_remote,
        None,
        &request_head,
        "public-draft-exact-fetch",
    ));

    state
        .metadata
        .mark_request_ready(MarkRequestReadyInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            actor_is_author: false,
            actor_can_mutate: false,
            stake_credits: Some(1),
            public_ready_count: 0,
            ready_queue_version: 0,
            event_id: "event_advertisement_ready".to_string(),
            stake_ledger_entry_id: Some("ledger_advertisement_ready".to_string()),
            now_unix: 4,
        })
        .await
        .unwrap();
    for (remote, bearer) in [
        (public_remote.as_str(), None),
        (permissioned_remote.as_str(), Some(author.as_str())),
        (permissioned_remote.as_str(), Some(invitee.as_str())),
        (permissioned_remote.as_str(), Some(maintainer.as_str())),
        (permissioned_remote.as_str(), Some(unrelated.as_str())),
    ] {
        assert!(advertises_request_ref(remote, bearer));
    }

    state
        .metadata
        .return_request_to_working(ReturnRequestToWorkingInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            actor_is_author: false,
            actor_is_maintainer: false,
            actor_can_mutate: false,
            reason: RequestReviewExitReason::AuthorReturned,
            event_id: "event_advertisement_working".to_string(),
            now_unix: 5,
        })
        .await
        .unwrap();
    assert!(advertises_request_ref(&permissioned_remote, Some(&author)));
    assert!(advertises_request_ref(&permissioned_remote, Some(&invitee)));
    assert!(!advertises_request_ref(
        &permissioned_remote,
        Some(&maintainer)
    ));
    assert!(!advertises_request_ref(
        &permissioned_remote,
        Some(&unrelated)
    ));
    assert!(!advertises_request_ref(&public_remote, None));
    for (remote, bearer, label) in [
        (
            public_remote.as_str(),
            None,
            "public-published-working-exact-fetch",
        ),
        (
            permissioned_remote.as_str(),
            Some(maintainer.as_str()),
            "maintainer-published-working-exact-fetch",
        ),
        (
            permissioned_remote.as_str(),
            Some(unrelated.as_str()),
            "unrelated-published-working-exact-fetch",
        ),
    ] {
        assert!(fetch_exact_request_tip(
            remote,
            bearer,
            &request_head,
            label
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hold_and_revocation_block_invitee_push_while_maintainer_override_invalidates() {
    let state = test_state_with_request().await;
    let (_author_checkout, _author_remote, author_server, _) =
        request_checkout(&state, "hold-invitee-source").await;
    insert_public_contributor(&state).await;
    state
        .metadata
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            target_handle: "contributor".to_string(),
            now_unix: 3,
        })
        .await
        .unwrap();
    let (invitee_checkout, invitee_remote, invitee_server) = request_push_checkout(
        &state,
        "held-invitee-push",
        CONTRIBUTOR_SUBJECT,
        CONTRIBUTOR_EMAIL,
    )
    .await;
    state
        .metadata
        .mark_request_ready(MarkRequestReadyInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            actor_is_author: false,
            actor_can_mutate: false,
            stake_credits: Some(10),
            public_ready_count: 0,
            ready_queue_version: 0,
            event_id: "event_hold_ready".to_string(),
            stake_ledger_entry_id: Some("ledger_hold_ready".to_string()),
            now_unix: 4,
        })
        .await
        .unwrap();
    state
        .metadata
        .set_request_hold(SetRequestHoldInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: test_owner_id(),
            actor_is_maintainer: false,
            held: true,
            event_id: "event_hold_invitee".to_string(),
            now_unix: 5,
        })
        .await
        .unwrap();
    push_change(
        &invitee_checkout,
        &invitee_remote,
        REQUEST_REF,
        "held-invitee.txt",
        "blocked while held\n",
        "held invitee change",
    )
    .unwrap_err();

    state
        .metadata
        .set_request_hold(SetRequestHoldInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: test_owner_id(),
            actor_is_maintainer: false,
            held: false,
            event_id: "event_release_before_revoke".to_string(),
            now_unix: 6,
        })
        .await
        .unwrap();
    state
        .metadata
        .remove_request_invitee(RemoveRequestInviteeCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            target_handle: "contributor".to_string(),
        })
        .await
        .unwrap();
    push_change(
        &invitee_checkout,
        &invitee_remote,
        REQUEST_REF,
        "revoked-invitee.txt",
        "blocked after revoke\n",
        "revoked invitee change",
    )
    .unwrap_err();
    drop(invitee_server);
    drop(author_server);

    insert_member_user(&state).await;
    state
        .metadata
        .set_request_hold(SetRequestHoldInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: test_owner_id(),
            actor_is_maintainer: false,
            held: true,
            event_id: "event_hold_maintainer_override".to_string(),
            now_unix: 7,
        })
        .await
        .unwrap();
    let (maintainer_checkout, maintainer_remote, _maintainer_server) =
        request_push_checkout(&state, "held-maintainer-push", MEMBER_SUBJECT, MEMBER_EMAIL).await;
    run_git(
        Some(&maintainer_checkout),
        &["fetch", &maintainer_remote, REQUEST_REF],
        "fetch held request for maintainer override",
    )
    .unwrap();
    run_git(
        Some(&maintainer_checkout),
        &["checkout", "-B", "maintainer-request", "FETCH_HEAD"],
        "checkout held request for maintainer override",
    )
    .unwrap();
    push_change(
        &maintainer_checkout,
        &maintainer_remote,
        REQUEST_REF,
        "maintainer-override.txt",
        "maintainer override\n",
        "held maintainer change",
    )
    .unwrap();
    let request = stored_request(&state, REQUEST_ID).await;
    assert_eq!(request.state, RequestState::Working);
    assert_eq!(request.held_at_unix, None);
    assert_eq!(
        state
            .metadata
            .credit_account_for_tests(&public_user_id())
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        100
    );
}

fn unrelated_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", UNRELATED_SUBJECT)
}

fn advertises_request_ref(remote: &str, bearer: Option<&str>) -> bool {
    let output = match bearer {
        Some(bearer) => {
            let header = format!("http.{remote}.extraHeader=Authorization: {bearer}");
            run_git_output(
                None,
                &["-c", &header, "ls-remote", remote],
                "reading request ref advertisement",
            )
        }
        None => run_git_output(
            None,
            &["ls-remote", remote],
            "reading public request ref advertisement",
        ),
    }
    .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .contains(REQUEST_REF)
}

fn fetch_exact_request_tip(
    remote: &str,
    bearer: Option<&str>,
    request_head: &str,
    label: &str,
) -> bool {
    let checkout = checkout_dir(label);
    run_git(
        None,
        &["init", checkout.to_str().unwrap()],
        "init exact fetch repo",
    )
    .unwrap();
    let header = bearer.map(|bearer| format!("http.{remote}.extraHeader=Authorization: {bearer}"));
    let output = match header.as_deref() {
        Some(header) => run_git_output(
            Some(&checkout),
            &["-c", header, "fetch", remote, request_head],
            "fetch exact request tip",
        ),
        None => run_git_output(
            Some(&checkout),
            &["fetch", remote, request_head],
            "fetch public exact request tip",
        ),
    }
    .unwrap();
    output.status.success()
}
