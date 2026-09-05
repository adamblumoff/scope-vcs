use super::*;

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    crate::persistence::ensure_private_dir(root.path()).unwrap();
    root
}
fn fixture(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    if relative.ends_with(".lock") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"lock").unwrap();
    } else {
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("private-object"), b"private").unwrap();
    }
    path
}
fn retired(root: &Path) -> Vec<PathBuf> {
    let key = format!("repo-{}", "a".repeat(64));
    vec![
        fixture(root, &format!("git-request-refs/{key}.git")),
        fixture(root, &format!("git-request-refs-locks/{key}-store.lock")),
        fixture(
            root,
            &format!("git-request-refs-locks/{key}-{}.lock", "b".repeat(64)),
        ),
        fixture(
            root,
            &format!("git-rx/{}-{}.git", "a".repeat(16), "b".repeat(32)),
        ),
        fixture(root, &format!("git-repos/{key}.git")),
        fixture(root, &format!("git-staged/{key}.git")),
    ]
}
#[test]
fn deletes_only_retired_storage_and_resumes_after_interruption() {
    let root = root();
    let old = retired(root.path());
    let current = [
        fixture(
            root.path(),
            &format!("git-request-refs/{}.git", "c".repeat(32)),
        ),
        fixture(
            root.path(),
            &format!("git-request-refs-locks/{}-store.lock", "c".repeat(32)),
        ),
        fixture(
            root.path(),
            &format!("git-rx/{}-{}.git", "c".repeat(32), "d".repeat(32)),
        ),
        fixture(root.path(), "git-cache/current"),
        fixture(root.path(), "git-segments/durable"),
        fixture(root.path(), "objects/snapshots"),
    ];
    assert!(open_writer(root.path()).is_err());
    assert!(scrub(root.path(), |_| bail!("interrupted")).is_err());
    assert!(!root.path().join(MARKER).exists());
    assert_eq!(old.iter().filter(|p| p.exists()).count(), 5);
    assert_eq!(scrub(root.path(), |_| Ok(())).unwrap(), 5);
    assert!(old.iter().all(|p| !p.exists()));
    assert!(current.iter().all(|p| p.exists()));
    assert!(complete(root.path()).unwrap());
    assert_eq!(scrub(root.path(), |_| Ok(())).unwrap(), 0);
}
#[test]
fn writers_and_scrub_exclude_each_other() {
    let root = root();
    let writer = open_writer(root.path()).unwrap();
    assert!(open_writer(root.path()).is_ok());
    assert!(scrub(root.path(), |_| Ok(())).is_err());
    drop(writer);
    let lock = open_lock(root.path()).unwrap();
    lock.try_lock().unwrap();
    assert!(open_writer(root.path()).is_err());
    drop(lock);
    assert_eq!(scrub(root.path(), |_| Ok(())).unwrap(), 0);
}
#[test]
fn ambiguous_target_blocks_entire_plan() {
    let root = root();
    let old = retired(root.path());
    fixture(root.path(), "git-request-refs/unknown.git");
    assert!(scrub(root.path(), |_| Ok(())).is_err());
    assert!(old.iter().all(|p| p.exists()));
    assert!(!root.path().join(MARKER).exists());
}
#[cfg(unix)]
#[test]
fn symlinks_and_out_of_root_paths_fail_closed() {
    use std::os::unix::fs::symlink;
    let root = root();
    let outside = tempfile::tempdir().unwrap();
    let old = retired(root.path());
    fs::write(outside.path().join("important"), b"keep").unwrap();
    symlink(outside.path(), old[0].join("escape")).unwrap();
    assert!(scrub(root.path(), |_| Ok(())).is_err());
    assert!(old.iter().all(|p| p.exists()));
    assert!(outside.path().join("important").exists());
    fs::remove_file(old[0].join("escape")).unwrap();
    let alias = outside.path().join("alias");
    symlink(root.path(), &alias).unwrap();
    assert!(scrub(&alias, |_| Ok(())).is_err());
    assert!(scrub(&root.path().join(".."), |_| Ok(())).is_err());
    symlink(outside.path(), root.path().join(MARKER)).unwrap();
    assert!(scrub(root.path(), |_| Ok(())).is_err());
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_groups_targets_and_nonprivate_roots() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for group in [
        "git-request-refs",
        "git-request-refs-locks",
        "git-rx",
        "git-repos",
        "git-staged",
    ] {
        let root = root();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join(group)).unwrap();
        assert!(scrub(root.path(), |_| Ok(())).is_err());
        assert!(!root.path().join(MARKER).exists());
    }
    let root = root();
    let outside = tempfile::tempdir().unwrap();
    let old = retired(root.path());
    fs::remove_dir_all(&old[0]).unwrap();
    symlink(outside.path(), &old[0]).unwrap();
    assert!(scrub(root.path(), |_| Ok(())).is_err());
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(scrub(root.path(), |_| Ok(())).is_err());
}

#[test]
fn resumes_when_deletion_finished_before_completion_was_recorded() {
    let root = root();
    let old = retired(root.path());
    assert!(
        scrub(root.path(), |count| {
            if count == 6 {
                bail!("crashed before marker");
            }
            Ok(())
        })
        .is_err()
    );
    assert!(old.iter().all(|path| !path.exists()));
    assert!(!root.path().join(MARKER).exists());
    fs::write(root.path().join(format!("{MARKER}.tmp")), b"partial").unwrap();
    assert_eq!(scrub(root.path(), |_| Ok(())).unwrap(), 0);
    assert!(complete(root.path()).unwrap());
}

#[cfg(unix)]
#[test]
fn initialization_releases_exclusive_lock_held_by_inherited_descriptor() {
    let root = root();
    let exclusive = open_lock(root.path()).unwrap();
    exclusive.try_lock().unwrap();
    // try_clone shares the same open file description and flock ownership as a
    // descriptor inherited across fork, without scheduling a child process.
    let inherited = exclusive.try_clone().unwrap();
    let writer = initialize_clean_writer(root.path(), exclusive).unwrap();
    assert!(complete(root.path()).unwrap());
    let another_writer = open_writer(root.path()).unwrap();
    assert!(scrub(root.path(), |_| Ok(())).is_err());
    drop(inherited);
    assert!(scrub(root.path(), |_| Ok(())).is_err());
    drop(another_writer);
    drop(writer);
    assert_eq!(scrub(root.path(), |_| Ok(())).unwrap(), 0);
}
