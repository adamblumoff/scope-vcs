use super::*;

#[test]
fn request_title_limit_accepts_max_and_rejects_max_plus_one() {
    let mut requests = BTreeMap::new();
    let mut accepted = public_start_input();
    accepted.title = Some("t".repeat(REQUEST_TITLE_MAX_BYTES));
    start_request(&mut requests, accepted).unwrap();

    let mut rejected_requests = BTreeMap::new();
    let mut rejected = public_start_input();
    rejected.title = Some("t".repeat(REQUEST_TITLE_MAX_BYTES + 1));
    let error = start_request(&mut rejected_requests, rejected).unwrap_err();

    assert_eq!(error.kind, crate::error::ErrorKind::BadRequest);
    assert!(rejected_requests.is_empty());
}

#[test]
fn timeline_body_limit_accepts_max_for_every_mutation() {
    let body = "x".repeat(REQUEST_TIMELINE_BODY_MAX_BYTES);

    let mut revision_requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut revision_events = BTreeMap::new();
    let mut revision = revision_input("head");
    revision.body = Some(body.clone());
    let revision =
        record_request_revision(&mut revision_requests, &mut revision_events, revision).unwrap();
    assert!(matches!(
        revision.event.payload,
        RequestEventPayload::RevisionPushed { note: Some(ref value), .. }
            if value.len() == REQUEST_TIMELINE_BODY_MAX_BYTES
    ));

    let mut needs_requests = BTreeMap::from([("req_1".to_string(), submitted_request())]);
    let mut needs_events = BTreeMap::new();
    let needs = mark_request_needs_response(
        &mut needs_requests,
        &mut needs_events,
        MarkRequestNeedsResponseInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            event_id: "event_needs".to_string(),
            body: body.clone(),
            now_unix: 30,
        },
    )
    .unwrap();
    assert!(matches!(
        needs.event.payload,
        RequestEventPayload::NeedsResponse { ref body, .. }
            if body.len() == REQUEST_TIMELINE_BODY_MAX_BYTES
    ));

    let mut response_request = submitted_request();
    response_request.state = RequestState::NeedsResponse;
    let mut response_requests = BTreeMap::from([("req_1".to_string(), response_request)]);
    let mut response_events = BTreeMap::new();
    let response = respond_to_request(
        &mut response_requests,
        &mut response_events,
        RespondToRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            event_id: "event_response".to_string(),
            body: Some(body.clone()),
            now_unix: 30,
        },
    )
    .unwrap();
    assert!(matches!(
        response.event.payload,
        RequestEventPayload::ContributorResponded { body: Some(ref value), .. }
            if value.len() == REQUEST_TIMELINE_BODY_MAX_BYTES
    ));

    let mut resolve_fixture = RequestFixture::submitted(0);
    let mut resolve = resolve_input(RequestDisposition::UsefulNotMerged);
    resolve.body = Some(body.clone());
    let resolve = resolve_fixture.resolve(resolve).unwrap();
    assert!(matches!(
        resolve.resolved_event.payload,
        RequestEventPayload::Resolved { body: Some(ref value), .. }
            if value.len() == REQUEST_TIMELINE_BODY_MAX_BYTES
    ));

    let mut merge_fixture = RequestFixture::submitted(0);
    let mut merge = clean_merge_input();
    merge.body = Some(body);
    let merge = merge_fixture.merge(merge).unwrap();
    assert!(matches!(
        merge.merged_event.payload,
        RequestEventPayload::Merged { body: Some(ref value), .. }
            if value.len() == REQUEST_TIMELINE_BODY_MAX_BYTES
    ));
}

#[test]
fn timeline_body_limit_rejects_max_plus_one_before_mutation() {
    let body = Some("x".repeat(REQUEST_TIMELINE_BODY_MAX_BYTES + 1));

    let revision_before = submitted_request();
    let mut revision_requests = BTreeMap::from([("req_1".to_string(), revision_before.clone())]);
    let mut revision_events = BTreeMap::new();
    let mut revision = revision_input("head");
    revision.body = body.clone();
    let error = record_request_revision(&mut revision_requests, &mut revision_events, revision)
        .unwrap_err();
    assert_eq!(error.kind, crate::error::ErrorKind::BadRequest);
    assert_eq!(revision_requests["req_1"], revision_before);
    assert!(revision_events.is_empty());

    let needs_before = submitted_request();
    let mut needs_requests = BTreeMap::from([("req_1".to_string(), needs_before.clone())]);
    let mut needs_events = BTreeMap::new();
    let error = mark_request_needs_response(
        &mut needs_requests,
        &mut needs_events,
        MarkRequestNeedsResponseInput {
            request_id: "req_1".to_string(),
            actor_user_id: "maintainer".to_string(),
            event_id: "event_needs".to_string(),
            body: body.clone().unwrap(),
            now_unix: 30,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, crate::error::ErrorKind::BadRequest);
    assert_eq!(needs_requests["req_1"], needs_before);
    assert!(needs_events.is_empty());

    let mut response_before = submitted_request();
    response_before.state = RequestState::NeedsResponse;
    let mut response_requests = BTreeMap::from([("req_1".to_string(), response_before.clone())]);
    let mut response_events = BTreeMap::new();
    let error = respond_to_request(
        &mut response_requests,
        &mut response_events,
        RespondToRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            event_id: "event_response".to_string(),
            body: body.clone(),
            now_unix: 30,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, crate::error::ErrorKind::BadRequest);
    assert_eq!(response_requests["req_1"], response_before);
    assert!(response_events.is_empty());

    let mut resolve_fixture = RequestFixture::submitted(0);
    let resolve_before = resolve_fixture.requests["req_1"].clone();
    let mut resolve = resolve_input(RequestDisposition::UsefulNotMerged);
    resolve.body = body.clone();
    let error = resolve_fixture.resolve(resolve).unwrap_err();
    assert_eq!(error.kind, crate::error::ErrorKind::BadRequest);
    assert_eq!(resolve_fixture.requests["req_1"], resolve_before);
    assert!(resolve_fixture.events.is_empty());
    assert!(resolve_fixture.ledger_entries.is_empty());

    let mut merge_fixture = RequestFixture::submitted(0);
    let merge_before = merge_fixture.requests["req_1"].clone();
    let mut merge = clean_merge_input();
    merge.body = body;
    let error = merge_fixture.merge(merge).unwrap_err();
    assert_eq!(error.kind, crate::error::ErrorKind::BadRequest);
    assert_eq!(merge_fixture.requests["req_1"], merge_before);
    assert!(merge_fixture.events.is_empty());
    assert!(merge_fixture.ledger_entries.is_empty());
}
