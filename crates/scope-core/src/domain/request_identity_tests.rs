use super::{
    requests::*,
    requests_tests::{ready_request, working_request},
};
use std::collections::BTreeMap;

#[test]
fn identity_edit_supports_each_field_combination_and_rejects_empty_or_unchanged_inputs() {
    let request = working_request();
    let original_description = request.description_markdown.clone();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut events = BTreeMap::new();

    let title_only = edit_request_identity(
        &mut requests,
        &mut events,
        EditRequestIdentityInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit_identity: true,
            event_id: "event_title".to_string(),
            title: Some("Focused title".to_string()),
            description_markdown: None,
            now_unix: 20,
        },
    )
    .unwrap();
    assert_eq!(title_only.request.title, "Focused title");
    assert_eq!(
        title_only.request.description_markdown,
        original_description
    );
    assert_eq!(title_only.event.kind, RequestEventKind::IdentityEdited);

    let description_only = edit_request_identity(
        &mut requests,
        &mut events,
        EditRequestIdentityInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit_identity: true,
            event_id: "event_description".to_string(),
            title: None,
            description_markdown: Some("Focused description".to_string()),
            now_unix: 21,
        },
    )
    .unwrap();
    assert_eq!(description_only.request.title, "Focused title");
    assert_eq!(
        description_only.request.description_markdown,
        "Focused description"
    );

    let empty = edit_request_identity(
        &mut requests,
        &mut events,
        EditRequestIdentityInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit_identity: true,
            event_id: "event_empty".to_string(),
            title: None,
            description_markdown: None,
            now_unix: 22,
        },
    )
    .unwrap_err();
    assert_eq!(empty.kind, crate::error::ErrorKind::BadRequest);

    let unchanged = edit_request_identity(
        &mut requests,
        &mut events,
        EditRequestIdentityInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit_identity: true,
            event_id: "event_unchanged".to_string(),
            title: Some("Focused title".to_string()),
            description_markdown: Some("Focused description".to_string()),
            now_unix: 22,
        },
    )
    .unwrap_err();
    assert_eq!(unchanged.kind, crate::error::ErrorKind::Conflict);
}

#[test]
fn held_request_rejects_identity_edits_at_domain_boundary() {
    let mut request = ready_request();
    request.held_at_unix = Some(21);
    request.held_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 21;
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let error = edit_request_identity(
        &mut requests,
        &mut BTreeMap::new(),
        EditRequestIdentityInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit_identity: true,
            event_id: "event_identity".to_string(),
            title: None,
            description_markdown: Some("Changed while held".to_string()),
            now_unix: 22,
        },
    )
    .unwrap_err();
    assert!(error.message.contains("while held"));
}

#[test]
fn ready_request_rejects_identity_edits_until_review_invalidation_exists() {
    let request = ready_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let error = edit_request_identity(
        &mut requests,
        &mut BTreeMap::new(),
        EditRequestIdentityInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit_identity: true,
            event_id: "event_identity".to_string(),
            title: None,
            description_markdown: Some("Changed while ready".to_string()),
            now_unix: 22,
        },
    )
    .unwrap_err();
    assert!(error.message.contains("while ready for review"));
}
