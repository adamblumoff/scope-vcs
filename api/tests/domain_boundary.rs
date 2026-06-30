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
        let compact_source = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        for layer in forbidden_layers {
            if has_forbidden_outer_layer_import(&compact_source, layer) {
                violations.push(format!("{} imports crate::{layer}", path.display()));
            }
        }
    });

    assert!(
        violations.is_empty(),
        "domain modules must not depend on outer layers:\n{}",
        violations.join("\n")
    );
}

fn has_forbidden_outer_layer_import(compact_source: &str, layer: &str) -> bool {
    let direct = format!("crate::{layer}::");
    let grouped_first = format!("crate::{{{layer}::");
    let grouped_later = format!(",{layer}::");

    compact_source.contains(&direct)
        || compact_source.contains(&grouped_first)
        || compact_source.contains(&grouped_later)
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
