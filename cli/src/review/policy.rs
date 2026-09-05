use super::tree::{ReviewNode, ReviewNodeKind, ReviewTree};
use scope_domain::{
    repo_config::RepoConfig,
    repo_visibility::{self, ToggleResult, VisibilityNodeKind, VisibilityTarget},
};

use scope_domain::repo_visibility::ReviewVisibility;

pub fn toggle_node_visibility(
    config: &mut RepoConfig,
    tree: &ReviewTree,
    node_id: usize,
) -> ToggleResult {
    repo_visibility::toggle_visibility_target(config, target_for_node(tree, node_id))
}

pub fn tree_visibilities(config: &RepoConfig, tree: &ReviewTree) -> Vec<ReviewVisibility> {
    let mut visibilities = vec![ReviewVisibility::Mixed; tree.nodes().len()];
    // Nodes are inserted after their parents, so every child summary is ready.
    for node in tree.nodes().iter().rev() {
        let children = if node.kind == ReviewNodeKind::File {
            &[][..]
        } else {
            node.children.as_slice()
        };
        visibilities[node.id] = children
            .iter()
            .map(|child| visibilities[*child])
            .reduce(ReviewVisibility::combine)
            .unwrap_or_else(|| {
                repo_visibility::target_visibility(
                    config,
                    &target_for_review_node(node, Vec::new()),
                )
            });
    }
    visibilities
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
