use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha1::{Digest, Sha1};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let snapshot_path = out_dir.join("repo_snapshot.rs");

    let snapshot = build_snapshot()
        .unwrap_or_else(|error| panic!("failed to build repository snapshot: {error}"));
    assert!(
        !snapshot.is_empty(),
        "repository snapshot must contain tracked files"
    );

    let mut source = String::from(
        "pub struct BuildRepoFile {\n    pub path: &'static str,\n    pub oid: &'static str,\n    pub content: &'static str,\n}\n\npub const BUILD_REPO_SNAPSHOT: &[BuildRepoFile] = &[\n",
    );
    for file in snapshot {
        source.push_str("    BuildRepoFile { path: ");
        source.push_str(&format!("{:?}", file.path));
        source.push_str(", oid: ");
        source.push_str(&format!("{:?}", file.oid));
        source.push_str(", content: ");
        source.push_str(&format!("{:?}", file.content));
        source.push_str(" },\n");
    }
    source.push_str("];\n");

    fs::write(snapshot_path, source).expect("write generated repository snapshot");
}

struct SnapshotFile {
    path: String,
    oid: String,
    content: String,
}

fn build_snapshot() -> Result<Vec<SnapshotFile>, String> {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|error| error.to_string())?);
    match build_git_snapshot(&manifest_dir) {
        Ok(snapshot) if !snapshot.is_empty() => Ok(snapshot),
        Ok(_) => {
            println!(
                "cargo:warning=Git index did not contain tracked files; using source snapshot"
            );
            build_filesystem_snapshot(&manifest_dir)
        }
        Err(git_error) => {
            println!("cargo:warning=failed to build Git snapshot: {git_error}");
            build_filesystem_snapshot(&manifest_dir)
                .map_err(|fallback_error| format!("{git_error}; {fallback_error}"))
        }
    }
}

fn build_git_snapshot(manifest_dir: &Path) -> Result<Vec<SnapshotFile>, String> {
    let repo_root = command_text(
        Command::new("git")
            .arg("-C")
            .arg(manifest_dir)
            .args(["rev-parse", "--show-toplevel"]),
    )?;

    println!(
        "cargo:rerun-if-changed={}",
        Path::new(&repo_root).join(".git/index").display()
    );

    let entries = git_tracked_file_entries(Path::new(&repo_root))?;
    let mut snapshot = Vec::new();
    for (path, oid) in entries {
        if is_internal_scope_path(&path) {
            continue;
        }

        println!(
            "cargo:rerun-if-changed={}",
            Path::new(&repo_root).join(&path).display()
        );
        let content = command_bytes(
            Command::new("git")
                .arg("-C")
                .arg(&repo_root)
                .args(["cat-file", "blob", &oid]),
        )?;
        let Ok(content) = String::from_utf8(content) else {
            println!("cargo:warning=skipping non-UTF-8 repository file {path}");
            continue;
        };

        snapshot.push(SnapshotFile {
            path: format!("/{path}"),
            oid,
            content,
        });
    }

    Ok(snapshot)
}

fn build_filesystem_snapshot(manifest_dir: &Path) -> Result<Vec<SnapshotFile>, String> {
    let repo_root = fallback_snapshot_root(manifest_dir);
    println!("cargo:rerun-if-changed={}", repo_root.display());

    let mut paths = Vec::new();
    collect_source_files(&repo_root, &repo_root, &mut paths)?;
    paths.sort();

    let mut snapshot = Vec::new();
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes =
            fs::read(&path).map_err(|error| format!("reading {}: {error}", path.display()))?;
        let Ok(content) = String::from_utf8(bytes.clone()) else {
            println!(
                "cargo:warning=skipping non-UTF-8 source file {}",
                path.display()
            );
            continue;
        };
        let relative = snapshot_path(&repo_root, &path)?;
        snapshot.push(SnapshotFile {
            path: format!("/{relative}"),
            oid: git_blob_oid(&bytes),
            content,
        });
    }

    if snapshot.is_empty() {
        return Err(format!(
            "source snapshot at {} did not contain UTF-8 files",
            repo_root.display()
        ));
    }

    Ok(snapshot)
}

fn fallback_snapshot_root(manifest_dir: &Path) -> PathBuf {
    let Some(crates_dir) = manifest_dir.parent() else {
        return manifest_dir.to_path_buf();
    };
    let Some(workspace_root) = crates_dir.parent() else {
        return manifest_dir.to_path_buf();
    };

    if workspace_root.join("Cargo.toml").is_file() && workspace_root.join("crates").is_dir() {
        workspace_root.to_path_buf()
    } else {
        manifest_dir.to_path_buf()
    }
}

fn collect_source_files(root: &Path, dir: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|error| format!("reading {}: {error}", dir.display()))? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("reading type for {}: {error}", path.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if file_type.is_dir() {
            if should_skip_dir(&name) {
                continue;
            }
            collect_source_files(root, &path, paths)?;
        } else if file_type.is_file() {
            let relative = snapshot_path(root, &path)?;
            if should_skip_file(&relative) {
                continue;
            }
            paths.push(path);
        }
    }

    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".scope" | ".turbo" | ".output" | "coverage" | "dist" | "node_modules" | "target"
    )
}

fn should_skip_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == ".env" || name.starts_with(".env.") || path.ends_with(".tmp")
}

fn snapshot_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|error| format!("building relative path for {}: {error}", path.display()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn git_blob_oid(content: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", content.len()).as_bytes());
    hasher.update(content);
    hex::encode(hasher.finalize())
}

fn git_tracked_file_entries(repo_root: &Path) -> Result<Vec<(String, String)>, String> {
    let output = command_bytes(
        Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["ls-files", "-z", "-s"]),
    )?;

    let mut entries = Vec::new();
    for raw in output.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let entry = std::str::from_utf8(raw).map_err(|error| error.to_string())?;
        let (metadata, path) = entry
            .split_once('\t')
            .ok_or_else(|| format!("unexpected git ls-files entry: {entry}"))?;
        let oid = metadata
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| format!("git entry is missing an object id: {entry}"))?;
        entries.push((path.replace('\\', "/"), oid.to_string()));
    }

    Ok(entries)
}

fn command_text(command: &mut Command) -> Result<String, String> {
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn command_bytes(command: &mut Command) -> Result<Vec<u8>, String> {
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(output.stdout)
}

fn is_internal_scope_path(path: &str) -> bool {
    path == ".scope" || path.starts_with(".scope/")
}
