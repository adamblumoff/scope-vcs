use super::{
    policy::{node_visibility, rule_label, toggle_node_visibility},
    tree::{ReviewNodeKind, ReviewTree},
};
use crate::git_repo::GitChangedPath;
use scope_domain::{
    repo_config::{HistoryRewriteAction, RepoConfig},
    repo_visibility::ReviewVisibility,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewMode {
    Standalone,
    Push,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ChangeListKind {
    Added,
    Deleted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReviewItem {
    ChangeSection(ChangeListKind),
    ChangePath(ChangeListKind, usize),
    TreeNode(usize),
}

#[derive(Clone, Debug)]
pub struct ReviewState {
    pub tree: ReviewTree,
    pub config: RepoConfig,
    original_config: RepoConfig,
    expanded_tree_nodes: BTreeSet<usize>,
    expanded_change_lists: BTreeSet<ChangeListKind>,
    added_paths: Vec<String>,
    deleted_paths: Vec<String>,
    visible_items: Vec<ReviewItem>,
    cursor: usize,
    scroll: usize,
    filter: String,
    editing_filter: bool,
    message: String,
    mode: ReviewMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewRow {
    ChangeSection {
        kind: ChangeListKind,
        count: usize,
        expanded: bool,
    },
    ChangePath {
        kind: ChangeListKind,
        path: String,
    },
    TreeNode {
        depth: usize,
        name: String,
        path: String,
        kind: ReviewNodeKind,
        expanded: bool,
        visibility: ReviewVisibility,
        rule: String,
        reserved: bool,
        change_status: Option<String>,
    },
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
        Self::new_with_changed_paths(tree, config, mode, &[])
    }

    pub fn new_with_changed_paths(
        tree: ReviewTree,
        config: RepoConfig,
        mode: ReviewMode,
        changed_paths: &[GitChangedPath],
    ) -> Self {
        let mut expanded_tree_nodes = BTreeSet::new();
        expanded_tree_nodes.insert(tree.root_id());
        let message = if config.history.rewrites.is_empty() {
            "Space toggles visibility. Right expands folders and change lists.".to_string()
        } else {
            format!(
                "{} history rewrite(s) in config. This review edits visibility only.",
                config.history.rewrites.len()
            )
        };
        let (added_paths, deleted_paths) = split_change_paths(changed_paths);
        let mut state = Self {
            tree,
            original_config: config.clone(),
            config,
            expanded_tree_nodes,
            expanded_change_lists: BTreeSet::new(),
            added_paths,
            deleted_paths,
            visible_items: Vec::new(),
            cursor: 0,
            scroll: 0,
            filter: String::new(),
            editing_filter: false,
            message,
            mode,
        };
        state.rebuild_visible_items();
        state.move_cursor_to_item(ReviewItem::TreeNode(state.tree.root_id()));
        state
    }

    pub fn is_dirty(&self) -> bool {
        self.config != self.original_config
    }

    pub fn mark_saved(&mut self) {
        self.original_config = self.config.clone();
        self.message = "Saved Scope repo config".to_string();
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

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn visible_rows(&self) -> Vec<ReviewRow> {
        self.visible_items
            .iter()
            .copied()
            .map(|item| self.row_for_item(item))
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
                self.rebuild_visible_items();
                self.message = "Filter cleared".to_string();
            }
            ReviewInput::Escape => return ReviewStateAction::Cancel,
            ReviewInput::Filter => {
                self.editing_filter = true;
                self.message = "Type to filter paths. Esc exits filter.".to_string();
            }
            ReviewInput::Help => {
                self.message =
                    "Arrows navigate/expand. Space toggles visibility or lists. / filters."
                        .to_string();
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
                self.rebuild_visible_items();
            }
            ReviewInput::Char(value) => {
                self.filter.push(value);
                self.rebuild_visible_items();
            }
            ReviewInput::Quit => {
                self.filter.push('q');
                self.rebuild_visible_items();
            }
            _ => {}
        }
        ReviewStateAction::None
    }

    fn move_cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_down(&mut self) {
        if self.cursor + 1 < self.visible_items.len() {
            self.cursor += 1;
        }
    }

    fn collapse_or_move_to_parent(&mut self) {
        let Some(item) = self.selected_item() else {
            return;
        };
        match item {
            ReviewItem::ChangeSection(kind) => {
                if self.expanded_change_lists.remove(&kind) {
                    self.rebuild_visible_items();
                }
            }
            ReviewItem::ChangePath(kind, _) => {
                self.move_cursor_to_item(ReviewItem::ChangeSection(kind));
            }
            ReviewItem::TreeNode(id) => self.collapse_tree_node_or_move_to_parent(id),
        }
    }

    fn collapse_tree_node_or_move_to_parent(&mut self, id: usize) {
        let node = self.tree.node(id);
        let kind = node.kind;
        let parent = node.parent;
        if kind != ReviewNodeKind::File
            && id != self.tree.root_id()
            && self.expanded_tree_nodes.remove(&id)
        {
            self.rebuild_visible_items();
            return;
        }
        if let Some(parent) = parent {
            self.move_cursor_to_item(ReviewItem::TreeNode(parent));
        }
    }

    fn expand_or_move_to_child(&mut self) {
        let Some(item) = self.selected_item() else {
            return;
        };
        match item {
            ReviewItem::ChangeSection(kind) => self.expand_change_list_or_move_to_first_path(kind),
            ReviewItem::ChangePath(_, _) => {}
            ReviewItem::TreeNode(id) => self.expand_tree_node_or_move_to_first_child(id),
        }
    }

    fn expand_change_list_or_move_to_first_path(&mut self, kind: ChangeListKind) {
        if self.expanded_change_lists.insert(kind) {
            self.rebuild_visible_items();
        } else if self.visible_items.get(self.cursor + 1).is_some_and(
            |item| matches!(item, ReviewItem::ChangePath(item_kind, _) if *item_kind == kind),
        ) {
            self.cursor += 1;
        }
    }

    fn expand_tree_node_or_move_to_first_child(&mut self, id: usize) {
        let node = self.tree.node(id);
        let kind = node.kind;
        let first_child = node.children.first().copied();
        if kind == ReviewNodeKind::File || first_child.is_none() {
            return;
        }
        if self.expanded_tree_nodes.insert(id) {
            self.rebuild_visible_items();
            return;
        }
        self.move_cursor_to_item(ReviewItem::TreeNode(
            first_child.expect("first child is checked above"),
        ));
    }

    fn toggle_selected(&mut self) {
        let Some(item) = self.selected_item() else {
            return;
        };
        match item {
            ReviewItem::ChangeSection(kind) => {
                if !self.expanded_change_lists.remove(&kind) {
                    self.expanded_change_lists.insert(kind);
                }
                self.rebuild_visible_items();
            }
            ReviewItem::ChangePath(_, _) => {
                self.message =
                    "Change lists are informational. Edit visibility in the file tree.".to_string();
            }
            ReviewItem::TreeNode(id) => {
                let result = toggle_node_visibility(&mut self.config, &self.tree, id);
                self.message = result.message;
            }
        }
    }

    fn selected_item(&self) -> Option<ReviewItem> {
        self.visible_items.get(self.cursor).copied()
    }

    fn move_cursor_to_item(&mut self, item: ReviewItem) {
        if let Some(index) = self
            .visible_items
            .iter()
            .position(|visible_item| *visible_item == item)
        {
            self.cursor = index;
        }
    }

    fn clamp_cursor(&mut self) {
        let visible_count = self.visible_items.len();
        if visible_count == 0 {
            self.cursor = 0;
        } else if self.cursor >= visible_count {
            self.cursor = visible_count - 1;
        }
    }

    fn rebuild_visible_items(&mut self) {
        let mut items = Vec::new();
        self.collect_change_items(ChangeListKind::Added, &mut items);
        self.collect_change_items(ChangeListKind::Deleted, &mut items);
        self.collect_visible_tree_items(self.tree.root_id(), &mut items);
        self.visible_items = items;
        self.clamp_cursor();
    }

    fn collect_change_items(&self, kind: ChangeListKind, items: &mut Vec<ReviewItem>) {
        let paths = self.paths_for(kind);
        let matching_indices = paths
            .iter()
            .enumerate()
            .filter(|(_, path)| self.filter.is_empty() || path.contains(&self.filter))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matching_indices.is_empty() {
            return;
        }

        items.push(ReviewItem::ChangeSection(kind));
        if !self.filter.is_empty() || self.expanded_change_lists.contains(&kind) {
            items.extend(
                matching_indices
                    .into_iter()
                    .map(|index| ReviewItem::ChangePath(kind, index)),
            );
        }
    }

    fn collect_visible_tree_items(&self, node_id: usize, items: &mut Vec<ReviewItem>) {
        let node = self.tree.node(node_id);
        if self.filter.is_empty()
            || node_id == self.tree.root_id()
            || node.path.contains(&self.filter)
            || node.name.contains(&self.filter)
        {
            items.push(ReviewItem::TreeNode(node_id));
        }
        if !self.filter.is_empty() || self.expanded_tree_nodes.contains(&node_id) {
            for child in &node.children {
                self.collect_visible_tree_items(*child, items);
            }
        }
    }

    fn row_for_item(&self, item: ReviewItem) -> ReviewRow {
        match item {
            ReviewItem::ChangeSection(kind) => ReviewRow::ChangeSection {
                kind,
                count: self.paths_for(kind).len(),
                expanded: !self.filter.is_empty() || self.expanded_change_lists.contains(&kind),
            },
            ReviewItem::ChangePath(kind, index) => ReviewRow::ChangePath {
                kind,
                path: self.paths_for(kind)[index].clone(),
            },
            ReviewItem::TreeNode(id) => {
                let node = self.tree.node(id);
                ReviewRow::TreeNode {
                    depth: node.depth,
                    name: node.name.clone(),
                    path: node.path.clone(),
                    kind: node.kind,
                    expanded: self.expanded_tree_nodes.contains(&id),
                    visibility: node_visibility(&self.config, &self.tree, id),
                    rule: rule_label(&self.config, node),
                    reserved: node.reserved,
                    change_status: node.change_status.clone(),
                }
            }
        }
    }

    fn paths_for(&self, kind: ChangeListKind) -> &[String] {
        match kind {
            ChangeListKind::Added => &self.added_paths,
            ChangeListKind::Deleted => &self.deleted_paths,
        }
    }
}

fn split_change_paths(changed_paths: &[GitChangedPath]) -> (Vec<String>, Vec<String>) {
    let mut added = changed_paths
        .iter()
        .filter(|path| path.status.starts_with('A'))
        .map(|path| path.path.clone())
        .collect::<Vec<_>>();
    let mut deleted = changed_paths
        .iter()
        .filter(|path| path.status.starts_with('D'))
        .map(|path| path.path.clone())
        .collect::<Vec<_>>();
    added.sort();
    deleted.sort();
    (added, deleted)
}

fn history_rewrite_action_label(action: HistoryRewriteAction) -> &'static str {
    match action {
        HistoryRewriteAction::RedactPublicHistory => "redact public history",
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
