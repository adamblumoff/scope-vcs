use super::*;
use scope_core::domain::repo_config::RepoConfig;

fn state_with_mode(mode: ReviewMode) -> ReviewState {
    let tree = ReviewTree::from_paths(&["src/lib.rs".to_string(), "README.md".to_string()], &[]);
    let config = RepoConfig::parse_json(
        br#"{
  "kind": "scope.repo-config",
  "version": 1,
  "visibility": {
    "default": "private",
    "rules": []
  },
  "history": { "rewrites": [] }
}"#,
    )
    .unwrap();
    ReviewState::new(tree, config, mode)
}

fn state() -> ReviewState {
    state_with_mode(ReviewMode::Standalone)
}

#[test]
fn right_arrow_expands_folder_and_moves_to_first_child_when_already_expanded() {
    let mut state = state();
    state.handle_input(ReviewInput::Down);
    assert_eq!(state.visible_rows()[state.cursor()].path, "/src");

    state.handle_input(ReviewInput::Right);
    assert!(
        state
            .visible_rows()
            .iter()
            .any(|row| row.path == "/src/lib.rs")
    );

    state.handle_input(ReviewInput::Right);
    assert_eq!(state.visible_rows()[state.cursor()].path, "/src/lib.rs");
}

#[test]
fn quit_with_dirty_config_requires_save_or_cancel() {
    let mut state = state();
    state.handle_input(ReviewInput::Toggle);

    assert_eq!(
        state.handle_input(ReviewInput::Quit),
        ReviewStateAction::None
    );
    assert!(state.message().contains("Unsaved changes"));
    assert_eq!(
        state.handle_input(ReviewInput::Escape),
        ReviewStateAction::Cancel
    );
}

#[test]
fn quit_cancels_push_review_even_when_clean() {
    let mut state = state_with_mode(ReviewMode::Push);

    assert_eq!(
        state.handle_input(ReviewInput::Quit),
        ReviewStateAction::Cancel
    );
}

#[test]
fn escape_clears_closed_filter_before_canceling() {
    let mut state = state();

    state.handle_input(ReviewInput::Filter);
    state.handle_input(ReviewInput::Char('s'));
    assert_eq!(state.filter(), "s");
    assert!(state.editing_filter());

    assert_eq!(
        state.handle_input(ReviewInput::Escape),
        ReviewStateAction::None
    );
    assert_eq!(state.filter(), "s");
    assert!(!state.editing_filter());

    assert_eq!(
        state.handle_input(ReviewInput::Escape),
        ReviewStateAction::None
    );
    assert_eq!(state.filter(), "");
    assert!(state.message().contains("Filter cleared"));

    assert_eq!(
        state.handle_input(ReviewInput::Escape),
        ReviewStateAction::Cancel
    );
}

#[test]
fn initial_message_surfaces_read_only_history_rewrites() {
    let tree = ReviewTree::from_paths(&["README.md".to_string()], &[]);
    let config = RepoConfig::parse_json(
        br#"{
  "kind": "scope.repo-config",
  "version": 1,
  "visibility": {
    "default": "private",
    "rules": []
  },
  "history": {
    "rewrites": [
      { "path": "/secret.txt", "action": "redact-public-history" }
    ]
  }
}"#,
    )
    .unwrap();

    let state = ReviewState::new(tree, config, ReviewMode::Push);

    assert!(state.message().contains("history rewrite"));
    assert_eq!(state.history_rewrite_count(), 1);
    assert_eq!(
        state.history_rewrite_summaries(),
        vec!["History rewrite: /secret.txt -> redact public history".to_string()]
    );
}

#[test]
fn deleted_paths_are_exposed_as_read_only_summaries() {
    let tree = ReviewTree::from_paths(&["README.md".to_string()], &[]);
    let config = RepoConfig::parse_json(
        br#"{
  "kind": "scope.repo-config",
  "version": 1,
  "visibility": {
    "default": "private",
    "rules": []
  },
  "history": { "rewrites": [] }
}"#,
    )
    .unwrap();

    let state = ReviewState::new_with_deleted_paths(
        tree,
        config,
        ReviewMode::Push,
        vec!["Deleted path: D old.txt".to_string()],
    );

    assert_eq!(
        state.deleted_path_summaries(),
        &["Deleted path: D old.txt".to_string()]
    );
}
