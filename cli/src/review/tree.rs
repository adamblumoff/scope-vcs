use crate::git_repo::GitChangedPath;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewTree {
    nodes: Vec<ReviewNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewNode {
    pub id: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub name: String,
    pub path: String,
    pub depth: usize,
    pub kind: ReviewNodeKind,
    pub reserved: bool,
    pub change_status: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewNodeKind {
    Root,
    Directory,
    File,
}

impl ReviewTree {
    pub fn from_paths(paths: &[String], changed_paths: &[GitChangedPath]) -> Self {
        let mut nodes = vec![ReviewNode {
            id: 0,
            parent: None,
            children: Vec::new(),
            name: ".".to_string(),
            path: "/".to_string(),
            depth: 0,
            kind: ReviewNodeKind::Root,
            reserved: false,
            change_status: None,
        }];
        let mut node_ids_by_path = BTreeMap::from([("/".to_string(), 0usize)]);
        let changed_statuses = changed_statuses_by_path(changed_paths);
        let unique_paths = paths.iter().map(String::as_str).collect::<BTreeSet<_>>();

        for relative_path in unique_paths {
            let parts = relative_path
                .split('/')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            let mut parent = 0usize;
            for index in 0..parts.len() {
                let path = format!("/{}", parts[..=index].join("/"));
                let is_file = index + 1 == parts.len();
                if let Some(existing_id) = node_ids_by_path.get(&path).copied() {
                    parent = existing_id;
                    continue;
                }

                let id = nodes.len();
                let kind = if is_file {
                    ReviewNodeKind::File
                } else {
                    ReviewNodeKind::Directory
                };
                nodes[parent].children.push(id);
                nodes.push(ReviewNode {
                    id,
                    parent: Some(parent),
                    children: Vec::new(),
                    name: parts[index].to_string(),
                    depth: index + 1,
                    reserved: is_reserved_scope_path(&path),
                    change_status: changed_statuses.get(&path).cloned(),
                    path: path.clone(),
                    kind,
                });
                node_ids_by_path.insert(path, id);
                parent = id;
            }
        }

        let mut tree = Self { nodes };
        tree.sort_children(0);
        tree
    }

    pub fn root_id(&self) -> usize {
        0
    }

    pub fn node(&self, id: usize) -> &ReviewNode {
        &self.nodes[id]
    }

    #[cfg(test)]
    pub fn nodes(&self) -> &[ReviewNode] {
        &self.nodes
    }

    pub fn file_paths_under(&self, id: usize) -> Vec<&str> {
        let node = self.node(id);
        if node.kind == ReviewNodeKind::File {
            return vec![node.path.as_str()];
        }

        let mut paths = Vec::new();
        self.collect_file_paths(id, &mut paths);
        paths
    }

    fn collect_file_paths<'a>(&'a self, id: usize, paths: &mut Vec<&'a str>) {
        let node = self.node(id);
        if node.kind == ReviewNodeKind::File {
            paths.push(node.path.as_str());
            return;
        }

        for child in &node.children {
            self.collect_file_paths(*child, paths);
        }
    }

    fn sort_children(&mut self, id: usize) {
        let children = self.nodes[id].children.clone();
        for child in &children {
            self.sort_children(*child);
        }
        let mut sorted_children = self.nodes[id].children.clone();
        sorted_children.sort_by(|left, right| {
            let left = &self.nodes[*left];
            let right = &self.nodes[*right];
            let left_rank = node_sort_rank(left.kind);
            let right_rank = node_sort_rank(right.kind);
            left_rank
                .cmp(&right_rank)
                .then_with(|| left.name.cmp(&right.name))
        });
        self.nodes[id].children = sorted_children;
    }
}

fn node_sort_rank(kind: ReviewNodeKind) -> u8 {
    match kind {
        ReviewNodeKind::Root => 0,
        ReviewNodeKind::Directory => 1,
        ReviewNodeKind::File => 2,
    }
}

fn changed_statuses_by_path(changed_paths: &[GitChangedPath]) -> BTreeMap<String, String> {
    changed_paths
        .iter()
        .map(|changed| {
            let path = changed
                .path
                .rsplit_once(" -> ")
                .map(|(_, new_path)| new_path)
                .unwrap_or(&changed.path);
            (format!("/{path}"), changed.status.clone())
        })
        .collect()
}

fn is_reserved_scope_path(path: &str) -> bool {
    path == "/.scope" || path.starts_with("/.scope/")
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
