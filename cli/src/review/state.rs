use super::{
    policy::{ReviewVisibility, node_visibility, rule_label, toggle_node_visibility},
    tree::{ReviewNodeKind, ReviewTree},
};
use scope_core::domain::repo_config::{HistoryRewriteAction, RepoConfig};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewMode {
    Standalone,
    Push,
}

#[derive(Clone, Debug)]
pub struct ReviewState {
    pub tree: ReviewTree,
    pub config: RepoConfig,
    original_config: RepoConfig,
    expanded: BTreeSet<usize>,
    cursor: usize,
    scroll: usize,
    filter: String,
    editing_filter: bool,
    message: String,
    deleted_path_summaries: Vec<String>,
    mode: ReviewMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewRow {
    pub id: usize,
    pub depth: usize,
    pub name: String,
    pub path: String,
    pub kind: ReviewNodeKind,
    pub expanded: bool,
    pub visibility: ReviewVisibility,
    pub rule: String,
    pub reserved: bool,
    pub change_status: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewInput {
    Up,
    Down,
    Left,
    Right,
    Toggle,
    Save,
    ContinuePush,
    Quit,
    Filter,
    Help,
    Escape,
    Backspace,
    Char(char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewStateAction {
    None,
    Save,
    ContinuePush,
    Exit,
    Cancel,
}

impl ReviewState {
    pub fn new(tree: ReviewTree, config: RepoConfig, mode: ReviewMode) -> Self {
        Self::new_with_deleted_paths(tree, config, mode, Vec::new())
    }

    pub fn new_with_deleted_paths(
        tree: ReviewTree,
        config: RepoConfig,
        mode: ReviewMode,
        deleted_path_summaries: Vec<String>,
    ) -> Self {
        let mut expanded = BTreeSet::new();
        expanded.insert(tree.root_id());
        let message = if config.history.rewrites.is_empty() {
            "Space toggles visibility. Right expands folders.".to_string()
        } else {
            format!(
                "{} history rewrite(s) in config. This review edits visibility only.",
                config.history.rewrites.len()
            )
        };
        Self {
            tree,
            original_config: config.clone(),
            config,
            expanded,
            cursor: 0,
            scroll: 0,
            filter: String::new(),
            editing_filter: false,
            message,
            deleted_path_summaries,
            mode,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.config != self.original_config
    }

    pub fn mark_saved(&mut self) {
        self.original_config = self.config.clone();
        self.message = "Saved .scope/repo.json".to_string();
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn editing_filter(&self) -> bool {
        self.editing_filter
    }

    pub fn mode(&self) -> ReviewMode {
        self.mode
    }

    pub fn history_rewrite_count(&self) -> usize {
        self.config.history.rewrites.len()
    }

    pub fn history_rewrite_summaries(&self) -> Vec<String> {
        self.config
            .history
            .rewrites
            .iter()
            .map(|rewrite| {
                format!(
                    "History rewrite: {} -> {}",
                    rewrite.path,
                    history_rewrite_action_label(rewrite.action)
                )
            })
            .collect()
    }

    pub fn deleted_path_summaries(&self) -> &[String] {
        &self.deleted_path_summaries
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn visible_rows(&self) -> Vec<ReviewRow> {
        self.visible_node_ids()
            .into_iter()
            .map(|id| {
                let node = self.tree.node(id);
                ReviewRow {
                    id,
                    depth: node.depth,
                    name: node.name.clone(),
                    path: node.path.clone(),
                    kind: node.kind,
                    expanded: self.expanded.contains(&id),
                    visibility: node_visibility(&self.config, &self.tree, id),
                    rule: rule_label(&self.config, node),
                    reserved: node.reserved,
                    change_status: node.change_status.clone(),
                }
            })
            .collect()
    }

    pub fn adjust_scroll(&mut self, viewport_height: usize) {
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        }
        if viewport_height > 0 && self.cursor >= self.scroll + viewport_height {
            self.scroll = self.cursor + 1 - viewport_height;
        }
    }

    pub fn handle_input(&mut self, input: ReviewInput) -> ReviewStateAction {
        if self.editing_filter {
            return self.handle_filter_input(input);
        }

        match input {
            ReviewInput::Up => self.move_cursor_up(),
            ReviewInput::Down => self.move_cursor_down(),
            ReviewInput::Left => self.collapse_or_move_to_parent(),
            ReviewInput::Right => self.expand_or_move_to_child(),
            ReviewInput::Toggle => self.toggle_selected(),
            ReviewInput::Save => return ReviewStateAction::Save,
            ReviewInput::ContinuePush if self.mode == ReviewMode::Push => {
                return ReviewStateAction::ContinuePush;
            }
            ReviewInput::Quit if self.mode == ReviewMode::Push => {
                return ReviewStateAction::Cancel;
            }
            ReviewInput::Quit => {
                return if self.is_dirty() {
                    self.message = "Unsaved changes. Press S to save or Esc to cancel.".to_string();
                    ReviewStateAction::None
                } else {
                    ReviewStateAction::Exit
                };
            }
            ReviewInput::Escape => return ReviewStateAction::Cancel,
            ReviewInput::Filter => {
                self.editing_filter = true;
                self.message = "Type to filter paths. Esc exits filter.".to_string();
            }
            ReviewInput::Help => {
                self.message = if self.mode == ReviewMode::Push {
                    "Keys: Space toggle, arrows move/expand, S save, P push, Q cancel, / filter"
                        .to_string()
                } else {
                    "Keys: Space toggle, arrows move/expand, S save, Q quit, / filter".to_string()
                };
            }
            ReviewInput::ContinuePush | ReviewInput::Backspace | ReviewInput::Char(_) => {}
        }
        ReviewStateAction::None
    }

    fn handle_filter_input(&mut self, input: ReviewInput) -> ReviewStateAction {
        match input {
            ReviewInput::Escape => {
                self.editing_filter = false;
                self.message = "Filter closed".to_string();
            }
            ReviewInput::Backspace => {
                self.filter.pop();
                self.clamp_cursor();
            }
            ReviewInput::Char(value) => {
                self.filter.push(value);
                self.clamp_cursor();
            }
            ReviewInput::Quit => {
                self.filter.push('q');
                self.clamp_cursor();
            }
            _ => {}
        }
        ReviewStateAction::None
    }

    fn move_cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_down(&mut self) {
        let rows = self.visible_node_ids();
        if self.cursor + 1 < rows.len() {
            self.cursor += 1;
        }
    }

    fn collapse_or_move_to_parent(&mut self) {
        let Some(id) = self.selected_node_id() else {
            return;
        };
        let node = self.tree.node(id);
        if node.kind != ReviewNodeKind::File
            && id != self.tree.root_id()
            && self.expanded.remove(&id)
        {
            self.clamp_cursor();
            return;
        }
        if let Some(parent) = node.parent {
            self.move_cursor_to_node(parent);
        }
    }

    fn expand_or_move_to_child(&mut self) {
        let Some(id) = self.selected_node_id() else {
            return;
        };
        let node = self.tree.node(id);
        if node.kind == ReviewNodeKind::File || node.children.is_empty() {
            return;
        }
        if self.expanded.insert(id) {
            return;
        }
        self.move_cursor_to_node(node.children[0]);
    }

    fn toggle_selected(&mut self) {
        let Some(id) = self.selected_node_id() else {
            return;
        };
        let result = toggle_node_visibility(&mut self.config, &self.tree, id);
        self.message = result.message;
    }

    fn selected_node_id(&self) -> Option<usize> {
        self.visible_node_ids().get(self.cursor).copied()
    }

    fn move_cursor_to_node(&mut self, node_id: usize) {
        if let Some(index) = self
            .visible_node_ids()
            .iter()
            .position(|visible_id| *visible_id == node_id)
        {
            self.cursor = index;
        }
    }

    fn clamp_cursor(&mut self) {
        let visible_count = self.visible_node_ids().len();
        if visible_count == 0 {
            self.cursor = 0;
        } else if self.cursor >= visible_count {
            self.cursor = visible_count - 1;
        }
    }

    fn visible_node_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.collect_visible_node_ids(self.tree.root_id(), &mut ids);
        ids
    }

    fn collect_visible_node_ids(&self, node_id: usize, ids: &mut Vec<usize>) {
        let node = self.tree.node(node_id);
        if self.filter.is_empty()
            || node_id == self.tree.root_id()
            || node.path.contains(&self.filter)
            || node.name.contains(&self.filter)
        {
            ids.push(node_id);
        }
        if !self.filter.is_empty() || self.expanded.contains(&node_id) {
            for child in &node.children {
                self.collect_visible_node_ids(*child, ids);
            }
        }
    }
}

fn history_rewrite_action_label(action: HistoryRewriteAction) -> &'static str {
    match action {
        HistoryRewriteAction::RedactPublicHistory => "redact public history",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_core::domain::repo_config::RepoConfig;

    fn state_with_mode(mode: ReviewMode) -> ReviewState {
        let tree =
            ReviewTree::from_paths(&["src/lib.rs".to_string(), "README.md".to_string()], &[]);
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
}
