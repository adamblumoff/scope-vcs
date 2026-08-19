use super::*;
use crate::git_repo::GitChangedPath;
use crate::repo_config::default_scope_repo_config;
use scope_domain::repo_config::{HistoryRewriteAction, HistoryRewriteRequest};

fn state_with_mode(mode: ReviewMode) -> ReviewState {
    let tree = ReviewTree::from_paths(&["src/lib.rs".to_string(), "README.md".to_string()], &[]);
    ReviewState::new(tree, default_scope_repo_config(), mode)
}

fn state() -> ReviewState {
    state_with_mode(ReviewMode::Standalone)
}

fn tree_path(row: &ReviewRow) -> Option<&str> {
    match row {
        ReviewRow::TreeNode { path, .. } => Some(path),
        ReviewRow::ChangeSection { .. } | ReviewRow::ChangePath { .. } => None,
    }
}

fn state_with_changes() -> ReviewState {
    let changed_paths = vec![
        GitChangedPath {
            status: "D".to_string(),
            path: "old.txt".to_string(),
        },
        GitChangedPath {
            status: "A".to_string(),
            path: "src/new.rs".to_string(),
        },
    ];
    let tree = ReviewTree::from_paths(
        &["src/new.rs".to_string(), "README.md".to_string()],
        &changed_paths,
    );
    ReviewState::new_with_changed_paths(
        tree,
        default_scope_repo_config(),
        ReviewMode::Push,
        &changed_paths,
    )
}

#[test]
fn right_arrow_expands_folder_and_moves_to_first_child_when_already_expanded() {
    let mut state = state();
    state.handle_input(ReviewInput::Down);
    assert_eq!(
        tree_path(&state.visible_rows()[state.cursor()]),
        Some("/src")
    );

    state.handle_input(ReviewInput::Right);
    assert!(
        state
            .visible_rows()
            .iter()
            .any(|row| tree_path(row) == Some("/src/lib.rs"))
    );

    state.handle_input(ReviewInput::Right);
    assert_eq!(
        tree_path(&state.visible_rows()[state.cursor()]),
        Some("/src/lib.rs")
    );
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
fn added_and_deleted_sections_start_collapsed() {
    let state = state_with_changes();
    let rows = state.visible_rows();

    assert!(matches!(
        rows[0],
        ReviewRow::ChangeSection {
            kind: ChangeListKind::Added,
            count: 1,
            expanded: false,
        }
    ));
    assert!(matches!(
        rows[1],
        ReviewRow::ChangeSection {
            kind: ChangeListKind::Deleted,
            count: 1,
            expanded: false,
        }
    ));
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, ReviewRow::ChangePath { .. }))
    );
    assert!(matches!(
        rows[state.cursor()],
        ReviewRow::TreeNode { ref path, .. } if path == "/"
    ));
}

#[test]
fn change_sections_expand_and_paths_remain_informational() {
    let mut state = state_with_changes();

    state.handle_input(ReviewInput::Up);
    state.handle_input(ReviewInput::Up);
    state.handle_input(ReviewInput::Right);
    assert!(matches!(
        state.visible_rows()[1],
        ReviewRow::ChangePath {
            kind: ChangeListKind::Added,
            ref path,
        } if path == "src/new.rs"
    ));

    state.handle_input(ReviewInput::Right);
    assert_eq!(state.cursor(), 1);
    state.handle_input(ReviewInput::Toggle);
    assert!(!state.is_dirty());
    assert!(state.message().contains("informational"));

    state.handle_input(ReviewInput::Left);
    assert_eq!(state.cursor(), 0);
    state.handle_input(ReviewInput::Left);
    assert!(
        !state
            .visible_rows()
            .iter()
            .any(|row| matches!(row, ReviewRow::ChangePath { .. }))
    );
}

#[test]
fn filtering_surfaces_matching_change_paths_without_persisting_expansion() {
    let mut state = state_with_changes();

    state.handle_input(ReviewInput::Filter);
    for value in "old".chars() {
        state.handle_input(ReviewInput::Char(value));
    }
    let rows = state.visible_rows();
    assert!(matches!(
        rows[0],
        ReviewRow::ChangeSection {
            kind: ChangeListKind::Deleted,
            expanded: true,
            ..
        }
    ));
    assert!(matches!(
        rows[1],
        ReviewRow::ChangePath {
            kind: ChangeListKind::Deleted,
            ref path,
        } if path == "old.txt"
    ));

    state.handle_input(ReviewInput::Escape);
    state.handle_input(ReviewInput::Escape);
    let rows = state.visible_rows();
    assert!(matches!(
        rows[1],
        ReviewRow::ChangeSection {
            kind: ChangeListKind::Deleted,
            expanded: false,
            ..
        }
    ));
}
