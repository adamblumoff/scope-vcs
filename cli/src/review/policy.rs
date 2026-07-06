use super::tree::{ReviewNode, ReviewNodeKind, ReviewTree};
use scope_core::domain::{
    repo_config::RepoConfig,
    repo_visibility::{self, ToggleResult, VisibilityNodeKind, VisibilityTarget},
};

pub use scope_core::domain::repo_visibility::{ReviewVisibility, visibility_label};

pub fn toggle_node_visibility(
    config: &mut RepoConfig,
    tree: &ReviewTree,
    node_id: usize,
) -> ToggleResult {
    repo_visibility::toggle_visibility_target(config, target_for_node(tree, node_id))
}

pub fn node_visibility(config: &RepoConfig, tree: &ReviewTree, node_id: usize) -> ReviewVisibility {
    repo_visibility::target_visibility(config, &target_for_node(tree, node_id))
}

pub fn rule_label(config: &RepoConfig, node: &ReviewNode) -> String {
    repo_visibility::rule_label(config, &target_for_review_node(node, Vec::new()))
}

fn target_for_node<'a>(tree: &'a ReviewTree, node_id: usize) -> VisibilityTarget<'a> {
    let node = tree.node(node_id);
    target_for_review_node(node, tree.file_paths_under(node_id))
}

fn target_for_review_node<'a>(
    node: &'a ReviewNode,
    file_paths_under: Vec<&'a str>,
) -> VisibilityTarget<'a> {
    VisibilityTarget {
        name: &node.name,
        path: &node.path,
        kind: visibility_node_kind(node.kind),
        reserved: node.reserved,
        file_paths_under,
    }
}

fn visibility_node_kind(kind: ReviewNodeKind) -> VisibilityNodeKind {
    match kind {
        ReviewNodeKind::Root => VisibilityNodeKind::Root,
        ReviewNodeKind::Directory => VisibilityNodeKind::Directory,
        ReviewNodeKind::File => VisibilityNodeKind::File,
    }
}
