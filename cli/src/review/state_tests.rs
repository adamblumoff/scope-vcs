use super::*;
use crate::repo_config::default_scope_repo_config;
use scope_core::domain::repo_config::{HistoryRewriteAction, HistoryRewriteRequest};

fn state_with_mode(mode: ReviewMode) -> ReviewState {
    let tree = ReviewTree::from_paths(&["src/lib.rs".to_string(), "README.md".to_string()], &[]);
    ReviewState::new(tree, default_scope_repo_config(), mode)
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
fn quit_respects_dirty_state_and_push_mode() {
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
    assert_eq!(
        state_with_mode(ReviewMode::Push).handle_input(ReviewInput::Quit),
        ReviewStateAction::Cancel,
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
    let mut config = default_scope_repo_config();
    config.history.rewrites.push(HistoryRewriteRequest {
        path: "/secret.txt".into(),
        action: HistoryRewriteAction::RedactPublicHistory,
    });

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

    let state = ReviewState::new_with_deleted_paths(
        tree,
        default_scope_repo_config(),
        ReviewMode::Push,
        vec!["Deleted path: D old.txt".to_string()],
    );

    assert_eq!(
        state.deleted_path_summaries(),
        &["Deleted path: D old.txt".to_string()]
    );
}
