use crate::requests::{
    CreateRequestRatingInput, Request, RequestActorRole, RequestAudience, RequestReputation,
    create_request_rating, eligible_rating_subject_user_id,
};

#[test]
fn reputation_accepts_only_possible_rating_totals() {
    assert_eq!(
        RequestReputation::from_totals(9, 2).unwrap(),
        RequestReputation {
            score_sum: 9,
            rating_count: 2,
        }
    );
    assert!(RequestReputation::from_totals(0, 0).is_ok());
    assert!(RequestReputation::from_totals(0, 1).is_err());
    assert!(RequestReputation::from_totals(6, 1).is_err());
    assert!(RequestReputation::from_totals(1, 0).is_err());
}

fn terminal_request(merged: bool) -> Request {
    Request {
        id: "request".to_string(),
        repo_id: "repo".to_string(),
        name: "change".to_string(),
        author_user_id: "author".to_string(),
        author_role: RequestActorRole::Public,
        audience: RequestAudience::Public,
        base_main_oid: "a".repeat(40),
        head_oid: "b".repeat(40),
        git_snapshot: None,
        title: "Change".to_string(),
        description_markdown: String::new(),
        activity_version: 3,
        submitted_at_unix: Some(2),
        closed_at_unix: (!merged).then_some(3),
        closed_by_user_id: (!merged).then(|| "maintainer".to_string()),
        merged_at_unix: merged.then_some(3),
        merged_by_user_id: merged.then(|| "maintainer".to_string()),
        merged_head_oid: merged.then(|| "b".repeat(40)),
        merged_main_oid: merged.then(|| "c".repeat(40)),
        created_at_unix: 1,
        updated_at_unix: 3,
    }
}

fn input(actor: &str, score: u8, reason: &str) -> CreateRequestRatingInput {
    CreateRequestRatingInput {
        id: format!("rating-{actor}"),
        request_id: "request".to_string(),
        actor_user_id: actor.to_string(),
        score,
        reason: reason.to_string(),
        now_unix: 4,
    }
}

#[test]
fn terminal_author_and_actor_can_rate_each_other_once() {
    for request in [terminal_request(false), terminal_request(true)] {
        let author_rating =
            create_request_rating(&request, &[], input("author", 5, "  solid  ")).unwrap();
        assert_eq!(author_rating.subject_user_id, "maintainer");
        assert_eq!(author_rating.reason, "solid");

        let ratings = vec![author_rating];
        assert!(eligible_rating_subject_user_id(&request, "author", &ratings).is_none());
        let maintainer_rating =
            create_request_rating(&request, &ratings, input("maintainer", 4, "Clear work"))
                .unwrap();
        assert_eq!(maintainer_rating.subject_user_id, "author");
    }
}

#[test]
fn ratings_reject_nonparticipants_nonterminal_self_and_invalid_content() {
    let request = terminal_request(false);
    assert!(create_request_rating(&request, &[], input("stranger", 5, "Fine")).is_err());
    assert!(create_request_rating(&request, &[], input("author", 0, "Fine")).is_err());
    assert!(create_request_rating(&request, &[], input("author", 5, "  ")).is_err());
    assert!(create_request_rating(&request, &[], input("author", 5, &"x".repeat(1025))).is_err());

    let mut open = request.clone();
    open.closed_at_unix = None;
    open.closed_by_user_id = None;
    assert!(create_request_rating(&open, &[], input("author", 5, "Fine")).is_err());

    let mut self_closed = request;
    self_closed.closed_by_user_id = Some("author".to_string());
    assert!(create_request_rating(&self_closed, &[], input("author", 5, "Fine")).is_err());
}
