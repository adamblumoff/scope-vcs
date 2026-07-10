use std::{fs, path::Path};

#[test]
fn domain_does_not_import_delivery_or_persistence_layers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain");
    let mut pending = vec![root];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                let source = fs::read_to_string(&path).unwrap();
                for layer in ["db", "git", "http", "persistence", "state"] {
                    assert!(
                        !source.contains(&format!("crate::{layer}"))
                            && !source.contains(&format!("crate::{{{layer}")),
                        "{} imports outer layer {layer}",
                        path.display()
                    );
                }
            }
        }
    }
}
