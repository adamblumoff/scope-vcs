use super::*;

#[test]
fn tree_sorts_folders_before_files_and_marks_reserved_scope_paths() {
    let tree = ReviewTree::from_paths(
        &[
            "README.md".to_string(),
            ".scope/repo.json".to_string(),
            "src/lib.rs".to_string(),
        ],
        &[],
    );

    let root_children = tree
        .node(tree.root_id())
        .children
        .iter()
        .map(|id| tree.node(*id).path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(root_children, vec!["/.scope", "/src", "/README.md"]);
    assert!(tree.nodes().iter().any(|node| {
        node.path == "/.scope/repo.json" && node.reserved && node.kind == ReviewNodeKind::File
    }));
}

#[test]
fn tree_maps_rename_status_to_new_path() {
    let tree = ReviewTree::from_paths(
        &["new.rs".to_string()],
        &[GitChangedPath {
            status: "R100".to_string(),
            path: "old.rs -> new.rs".to_string(),
        }],
    );

    let file = tree
        .nodes()
        .iter()
        .find(|node| node.path == "/new.rs")
        .unwrap();
    assert_eq!(file.change_status.as_deref(), Some("R100"));
}
