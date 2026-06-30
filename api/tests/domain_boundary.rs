use std::{
    fs,
    path::{Path, PathBuf},
};

#[test]
fn domain_modules_do_not_import_outer_layers() {
    let domain_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain");
    let forbidden_layers = ["git", "http", "state", "db", "persistence"];
    let mut violations = Vec::new();

    visit_rust_files(&domain_dir, &mut |path| {
        let source = fs::read_to_string(path).expect("domain source should be readable");
        violations.extend(forbidden_outer_layer_imports(
            path,
            &source,
            &forbidden_layers,
        ));
    });

    assert!(
        violations.is_empty(),
        "domain modules must not depend on outer layers:\n{}",
        violations.join("\n")
    );
}

fn forbidden_outer_layer_imports(
    path: &Path,
    source: &str,
    forbidden_layers: &[&str],
) -> Vec<String> {
    let mut violations = Vec::new();

    for use_statement in use_statements(source) {
        let compact_statement = use_statement
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        if !compact_statement.starts_with("usecrate::") {
            continue;
        }

        for layer in forbidden_layers {
            if has_forbidden_outer_layer_import(&compact_statement, layer) {
                violations.push(format!("{} imports crate::{layer}", path.display()));
            }
        }
    }

    violations
}

fn use_statements(source: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current_statement = None::<String>;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if current_statement.is_none() && !trimmed.starts_with("use ") {
            continue;
        }

        let statement = current_statement.get_or_insert_with(String::new);
        statement.push_str(trimmed);
        statement.push('\n');

        if trimmed.ends_with(';') {
            statements.push(
                current_statement
                    .take()
                    .expect("statement exists after insertion"),
            );
        }
    }

    statements
}

fn has_forbidden_outer_layer_import(compact_statement: &str, layer: &str) -> bool {
    let direct = format!("crate::{layer}::");
    let grouped_first = format!("crate::{{{layer}::");
    let grouped_later = format!(",{layer}::");

    compact_statement.contains(&direct)
        || compact_statement.contains(&grouped_first)
        || compact_statement.contains(&grouped_later)
}

fn visit_rust_files(dir: &Path, visitor: &mut impl FnMut(&Path)) {
    let mut entries = fs::read_dir(dir)
        .expect("domain directory should be readable")
        .map(|entry| {
            entry
                .expect("domain directory entry should be readable")
                .path()
        })
        .collect::<Vec<PathBuf>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            visit_rust_files(&path, visitor);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            visitor(&path);
        }
    }
}

#[test]
fn boundary_import_detection_ignores_comments_and_strings() {
    let source = r#"
// Previously called crate::state::live_tree.
const NOTE: &str = "crate::git::import";
"#;

    let violations = forbidden_outer_layer_imports(
        Path::new("example.rs"),
        source,
        &["git", "http", "state", "db", "persistence"],
    );

    assert!(violations.is_empty());
}

#[test]
fn boundary_import_detection_catches_grouped_outer_imports() {
    let source = r#"
use crate::{
    domain::store::StoredRepository,
    git::import::ReceivePackUpdate,
};
"#;

    let violations = forbidden_outer_layer_imports(
        Path::new("example.rs"),
        source,
        &["git", "http", "state", "db", "persistence"],
    );

    assert_eq!(violations, vec!["example.rs imports crate::git"]);
}
