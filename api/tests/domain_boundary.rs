use std::{
    fs,
    path::{Path, PathBuf},
};

#[test]
fn domain_modules_do_not_import_outer_layers() {
    let domain_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/scope-core/src/domain");
    let forbidden_layers = ["git", "http", "state", "db", "persistence"];
    let mut violations = Vec::new();

    visit_rust_files(&domain_dir, &mut |path| {
        let source = fs::read_to_string(path).expect("domain source should be readable");
        violations.extend(forbidden_outer_layer_references(
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

fn forbidden_outer_layer_references(
    path: &Path,
    source: &str,
    forbidden_layers: &[&str],
) -> Vec<String> {
    let mut violations = Vec::new();
    let sanitized_source = strip_comments_and_strings(source);
    let compact_source = compact(&sanitized_source);

    for layer in forbidden_layers {
        if direct_outer_layer_reference_patterns(layer)
            .iter()
            .any(|pattern| compact_source.contains(pattern))
        {
            violations.push(format!("{} references crate::{layer}", path.display()));
        }
    }

    for use_statement in use_statements(&sanitized_source) {
        let compact_statement = compact(&use_statement);
        for layer in forbidden_layers {
            if has_forbidden_outer_layer_import(&compact_statement, layer) {
                violations.push(format!("{} references crate::{layer}", path.display()));
            }
        }
    }

    violations.sort();
    violations.dedup();
    violations
}

fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn use_statements(source: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current_statement = None::<String>;

    for line in source.lines() {
        let trimmed = line.trim_start();
        let statement_line = if current_statement.is_some() {
            trimmed
        } else if let Some(use_start) = use_statement_start(trimmed) {
            use_start
        } else {
            continue;
        };

        let statement = current_statement.get_or_insert_with(String::new);
        statement.push_str(statement_line);
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

fn use_statement_start(trimmed_line: &str) -> Option<&str> {
    if trimmed_line.starts_with("use ") {
        return Some(trimmed_line);
    }

    let after_pub = trimmed_line.strip_prefix("pub")?;
    if after_pub.chars().next().is_some_and(char::is_whitespace) {
        let after_visibility = after_pub.trim_start();
        return after_visibility
            .starts_with("use ")
            .then_some(after_visibility);
    }

    let after_scope_start = after_pub.strip_prefix('(')?;
    let scope_end = after_scope_start.find(')')?;
    let after_visibility = after_scope_start[scope_end + 1..].trim_start();
    after_visibility
        .starts_with("use ")
        .then_some(after_visibility)
}

fn strip_comments_and_strings(source: &str) -> String {
    let chars = source.chars().collect::<Vec<_>>();
    let mut stripped = String::with_capacity(source.len());
    let mut index = 0;

    while index < chars.len() {
        if starts_with(&chars, index, &['/', '/']) {
            push_replacement(&mut stripped, chars[index]);
            push_replacement(&mut stripped, chars[index + 1]);
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                push_replacement(&mut stripped, chars[index]);
                index += 1;
            }
            continue;
        }

        if starts_with(&chars, index, &['/', '*']) {
            push_replacement(&mut stripped, chars[index]);
            push_replacement(&mut stripped, chars[index + 1]);
            index += 2;
            while index < chars.len() {
                let current = chars[index];
                let is_block_end = starts_with(&chars, index, &['*', '/']);
                push_replacement(&mut stripped, current);
                index += 1;
                if is_block_end {
                    push_replacement(&mut stripped, chars[index]);
                    index += 1;
                    break;
                }
            }
            continue;
        }

        if let Some(raw_string_end) = raw_string_end(&chars, index) {
            while index < raw_string_end {
                push_replacement(&mut stripped, chars[index]);
                index += 1;
            }
            continue;
        }

        if let Some(char_literal_end) = char_literal_end(&chars, index) {
            while index < char_literal_end {
                push_replacement(&mut stripped, chars[index]);
                index += 1;
            }
            continue;
        }

        if chars[index] == '"' {
            push_replacement(&mut stripped, chars[index]);
            index += 1;
            let mut escaped = false;
            while index < chars.len() {
                let current = chars[index];
                push_replacement(&mut stripped, current);
                index += 1;
                if current == '"' && !escaped {
                    break;
                }
                escaped = current == '\\' && !escaped;
                if current != '\\' {
                    escaped = false;
                }
            }
            continue;
        }

        stripped.push(chars[index]);
        index += 1;
    }

    stripped
}

fn starts_with(chars: &[char], index: usize, pattern: &[char]) -> bool {
    chars
        .get(index..index.saturating_add(pattern.len()))
        .is_some_and(|slice| slice == pattern)
}

fn push_replacement(output: &mut String, character: char) {
    if character == '\n' {
        output.push('\n');
    } else {
        output.push(' ');
    }
}

fn raw_string_end(chars: &[char], start: usize) -> Option<usize> {
    let mut cursor = start;
    if chars.get(cursor) == Some(&'b') {
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'r') {
        return None;
    }
    cursor += 1;

    let mut hashes = 0;
    while chars.get(cursor) == Some(&'#') {
        hashes += 1;
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'"') {
        return None;
    }
    cursor += 1;

    while cursor < chars.len() {
        if chars[cursor] == '"' {
            let hash_end = cursor + 1 + hashes;
            if hash_end <= chars.len()
                && chars[cursor + 1..hash_end]
                    .iter()
                    .all(|character| *character == '#')
            {
                return Some(hash_end);
            }
        }
        cursor += 1;
    }

    Some(chars.len())
}

fn char_literal_end(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start) != Some(&'\'') {
        return None;
    }

    let character = *chars.get(start + 1)?;
    if character == '\n' {
        return None;
    }

    if character != '\\' {
        return (chars.get(start + 2) == Some(&'\'')).then_some(start + 3);
    }

    let escaped = *chars.get(start + 2)?;
    let mut cursor = start + 3;
    if escaped == 'u' && chars.get(cursor) == Some(&'{') {
        cursor += 1;
        while cursor < chars.len() && chars[cursor] != '}' {
            if chars[cursor] == '\n' {
                return None;
            }
            cursor += 1;
        }
        if chars.get(cursor) != Some(&'}') {
            return None;
        }
        cursor += 1;
    } else if escaped == 'x' {
        cursor += 2;
    }

    (chars.get(cursor) == Some(&'\'')).then_some(cursor + 1)
}

fn has_forbidden_outer_layer_import(compact_statement: &str, layer: &str) -> bool {
    if import_from_root_references_layer(compact_statement, "crate", layer) {
        return true;
    }

    for depth in 1..=4 {
        if import_from_root_references_layer(compact_statement, &relative_root(depth), layer) {
            return true;
        }
    }

    false
}

fn import_from_root_references_layer(compact_statement: &str, root: &str, layer: &str) -> bool {
    let prefix = format!("use{root}::");
    let Some(import_path) = compact_statement.strip_prefix(&prefix) else {
        return false;
    };

    starts_with_layer_segment(import_path, layer)
        || import_path
            .strip_prefix('{')
            .is_some_and(|group| grouped_import_references_layer(group, layer))
}

fn grouped_import_references_layer(group: &str, layer: &str) -> bool {
    starts_with_layer_segment(group, layer)
        || group.match_indices(',').any(|(index, _)| {
            let next_segment = &group[index + 1..];
            starts_with_layer_segment(next_segment, layer)
        })
}

fn starts_with_layer_segment(import_path: &str, layer: &str) -> bool {
    let Some(remainder) = import_path.strip_prefix(layer) else {
        return false;
    };

    remainder.starts_with("::")
        || remainder.starts_with(';')
        || remainder.starts_with(',')
        || remainder.starts_with('}')
        || remainder.starts_with("as")
}

fn direct_outer_layer_reference_patterns(layer: &str) -> Vec<String> {
    let mut patterns = vec![format!("crate::{layer}::")];

    for depth in 1..=4 {
        patterns.push(format!("{}::{layer}::", relative_root(depth)));
    }

    patterns
}

fn relative_root(depth: usize) -> String {
    std::iter::repeat_n("super", depth)
        .collect::<Vec<_>>()
        .join("::")
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
    let source = r##"
// Previously called crate::state::live_tree.
const NOTE: &str = "crate::git::import";
const RAW: &str = r#"crate::http::responses"#;
"##;

    let violations = forbidden_outer_layer_references(
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

    let violations = forbidden_outer_layer_references(
        Path::new("example.rs"),
        source,
        &["git", "http", "state", "db", "persistence"],
    );

    assert_eq!(violations, vec!["example.rs references crate::git"]);
}

#[test]
fn boundary_import_detection_catches_bare_and_alias_outer_imports() {
    let source = r#"
use crate::git as outer_git;
use crate::{domain::store::StoredRepository, state};
"#;

    let violations = forbidden_outer_layer_references(
        Path::new("example.rs"),
        source,
        &["git", "http", "state", "db", "persistence"],
    );

    assert_eq!(
        violations,
        vec![
            "example.rs references crate::git",
            "example.rs references crate::state"
        ]
    );
}

#[test]
fn boundary_import_detection_catches_relative_outer_imports() {
    let source = r#"
use super::super::git::import::ReceivePackUpdate;
"#;

    let violations = forbidden_outer_layer_references(
        Path::new("example.rs"),
        source,
        &["git", "http", "state", "db", "persistence"],
    );

    assert_eq!(violations, vec!["example.rs references crate::git"]);
}

#[test]
fn boundary_import_detection_catches_visibility_qualified_outer_imports() {
    let source = r#"
pub(crate) use crate::git as outer_git;
pub(in crate::domain) use crate::{state};
"#;

    let violations = forbidden_outer_layer_references(
        Path::new("example.rs"),
        source,
        &["git", "http", "state", "db", "persistence"],
    );

    assert_eq!(
        violations,
        vec![
            "example.rs references crate::git",
            "example.rs references crate::state"
        ]
    );
}

#[test]
fn boundary_detection_catches_direct_outer_references() {
    let source = r#"
fn preview(repo: &mut StoredRepository, staged: StagedRepoUpdate) {
    crate::git::import::apply_receive_pack_update(repo, staged).unwrap();
}
"#;

    let violations = forbidden_outer_layer_references(
        Path::new("example.rs"),
        source,
        &["git", "http", "state", "db", "persistence"],
    );

    assert_eq!(violations, vec!["example.rs references crate::git"]);
}

#[test]
fn boundary_detection_catches_reference_after_quote_char_literal() {
    let source = r#"
const QUOTE: char = '"';

fn preview(repo: &mut StoredRepository, staged: StagedRepoUpdate) {
    crate::git::import::apply_receive_pack_update(repo, staged).unwrap();
}
"#;

    let violations = forbidden_outer_layer_references(
        Path::new("example.rs"),
        source,
        &["git", "http", "state", "db", "persistence"],
    );

    assert_eq!(violations, vec!["example.rs references crate::git"]);
}
