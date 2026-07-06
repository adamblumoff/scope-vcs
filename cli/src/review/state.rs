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
    visible_ids: Vec<usize>,
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
        let mut state = Self {
            tree,
            original_config: config.clone(),
            config,
            expanded,
            visible_ids: Vec::new(),
            cursor: 0,
            scroll: 0,
            filter: String::new(),
            editing_filter: false,
            message,
            deleted_path_summaries,
            mode,
        };
        state.rebuild_visible_ids();
        state
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
        self.visible_ids
            .iter()
            .copied()
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
            ReviewInput::Escape if !self.filter.is_empty() => {
                self.filter.clear();
                self.rebuild_visible_ids();
                self.message = "Filter cleared".to_string();
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
                self.rebuild_visible_ids();
            }
            ReviewInput::Char(value) => {
                self.filter.push(value);
                self.rebuild_visible_ids();
            }
            ReviewInput::Quit => {
                self.filter.push('q');
                self.rebuild_visible_ids();
            }
            _ => {}
        }
        ReviewStateAction::None
    }

    fn move_cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_down(&mut self) {
        if self.cursor + 1 < self.visible_ids.len() {
            self.cursor += 1;
        }
    }

    fn collapse_or_move_to_parent(&mut self) {
        let Some(id) = self.selected_node_id() else {
            return;
        };
        let node = self.tree.node(id);
        let kind = node.kind;
        let parent = node.parent;
        if kind != ReviewNodeKind::File && id != self.tree.root_id() && self.expanded.remove(&id) {
            self.rebuild_visible_ids();
            return;
        }
        if let Some(parent) = parent {
            self.move_cursor_to_node(parent);
        }
    }

    fn expand_or_move_to_child(&mut self) {
        let Some(id) = self.selected_node_id() else {
            return;
        };
        let node = self.tree.node(id);
        let kind = node.kind;
        let first_child = node.children.first().copied();
        if kind == ReviewNodeKind::File || first_child.is_none() {
            return;
        }
        if self.expanded.insert(id) {
            self.rebuild_visible_ids();
            return;
        }
        self.move_cursor_to_node(first_child.expect("first child is checked above"));
    }

    fn toggle_selected(&mut self) {
        let Some(id) = self.selected_node_id() else {
            return;
        };
        let result = toggle_node_visibility(&mut self.config, &self.tree, id);
        self.message = result.message;
    }

    fn selected_node_id(&self) -> Option<usize> {
        self.visible_ids.get(self.cursor).copied()
    }

    fn move_cursor_to_node(&mut self, node_id: usize) {
        if let Some(index) = self
            .visible_ids
            .iter()
            .position(|visible_id| *visible_id == node_id)
        {
            self.cursor = index;
        }
    }

    fn clamp_cursor(&mut self) {
        let visible_count = self.visible_ids.len();
        if visible_count == 0 {
            self.cursor = 0;
        } else if self.cursor >= visible_count {
            self.cursor = visible_count - 1;
        }
    }

    fn rebuild_visible_ids(&mut self) {
        let mut ids = Vec::new();
        self.collect_visible_node_ids(self.tree.root_id(), &mut ids);
        self.visible_ids = ids;
        self.clamp_cursor();
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
#[path = "state_tests.rs"]
mod tests;
