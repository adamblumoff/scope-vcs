use super::{
    archive::{BoundedWriter, create_archive},
    finalize::save_cache,
    identity::{MAX_CACHE_KEY_FILE_BYTES, digest_inputs_at, open_key_file},
    restore::CachePreparationPhases,
    types::{CacheFinalizationOutcome, PreparedCache},
};
use crate::api::RuntimeClient;
use scope_cache_domain::MAX_CACHE_OBJECT_BYTES;
use scope_domain::runs::cache::definition::CacheKeyInputs;
use std::{
    collections::BTreeMap,
    fs,
    io::Write as _,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[test]
fn archives_are_identical_across_creation_order_and_metadata() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    populate_cache(first.path(), false, Duration::from_secs(10));
    populate_cache(second.path(), true, Duration::from_secs(20));
    let first_archive = tempfile::NamedTempFile::new().unwrap();
    let second_archive = tempfile::NamedTempFile::new().unwrap();

    let first_identity = create_archive(first.path(), first_archive.path()).unwrap();
    let second_identity = create_archive(second.path(), second_archive.path()).unwrap();

    let first_bytes = fs::read(first_archive.path()).unwrap();
    let second_bytes = fs::read(second_archive.path()).unwrap();
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_identity, second_identity);
    use sha2::{Digest as _, Sha256};
    assert_eq!(
        first_identity,
        (
            first_bytes.len() as u64,
            hex::encode(Sha256::digest(&first_bytes))
        )
    );
}

#[test]
fn archives_have_sorted_paths_and_normalized_headers() {
    let source = tempfile::tempdir().unwrap();
    populate_cache(source.path(), true, Duration::from_secs(30));
    let output = tempfile::NamedTempFile::new().unwrap();
    create_archive(source.path(), output.path()).unwrap();

    let decoder = zstd::Decoder::new(fs::File::open(output.path()).unwrap()).unwrap();
    let mut archive = tar::Archive::new(decoder);
    let headers = archive
        .entries()
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let header = entry.header();
            (
                entry.path().unwrap().into_owned(),
                header.mode().unwrap(),
                header.uid().unwrap(),
                header.gid().unwrap(),
                header.mtime().unwrap(),
            )
        })
        .collect::<Vec<_>>();

    let paths = headers
        .iter()
        .map(|(path, ..)| path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        ["bin", "bin/run", "data.txt", "run-link"]
            .map(PathBuf::from)
            .to_vec()
    );
    assert!(
        headers
            .iter()
            .all(|(_, _, uid, gid, mtime)| (*uid, *gid, *mtime) == (0, 0, 0))
    );
    let modes = headers
        .into_iter()
        .map(|(path, mode, ..)| (path, mode))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(modes[Path::new("bin")], 0o755);
    assert_eq!(modes[Path::new("bin/run")], 0o755);
    assert_eq!(modes[Path::new("data.txt")], 0o644);
    assert_eq!(modes[Path::new("run-link")], 0o777);
}

#[test]
fn bounded_writer_accepts_the_limit_and_rejects_the_next_byte() {
    let mut writer = BoundedWriter::new(Vec::new(), 4);
    writer.write_all(b"four").unwrap();
    let error = writer.write_all(b"!").unwrap_err();
    assert_eq!(error.to_string(), "cache archive exceeds 4 bytes");
    assert_eq!(writer.written, 4);
    assert_eq!(MAX_CACHE_OBJECT_BYTES, 1024 * 1024 * 1024);
}

#[test]
fn archive_hash_counts_only_bytes_accepted_by_partial_writes() {
    struct PartialWriter;
    impl std::io::Write for PartialWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len().min(2))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BoundedWriter::new(PartialWriter, 8);
    writer.write_all(b"complete").unwrap();
    use sha2::{Digest as _, Sha256};
    assert_eq!(
        writer.identity(),
        (8, hex::encode(Sha256::digest(b"complete")))
    );
}

#[test]
fn cache_preparation_total_is_derived_from_timed_phases() {
    let phases = CachePreparationPhases {
        key_ms: 1,
        metadata_ms: 2,
        size_bytes: u64::MAX,
        download_verify_ms: 3,
        sync_ms: 4,
        extraction_ms: 5,
    };

    assert_eq!(phases.prepare_ms(), 15);
    assert_eq!(
        CachePreparationPhases {
            key_ms: u64::MAX,
            metadata_ms: 1,
            ..CachePreparationPhases::default()
        }
        .prepare_ms(),
        u64::MAX
    );
}

#[test]
fn exact_hit_skips_archive_hash_and_upload() {
    let cache = PreparedCache {
        exact_digest: "a".repeat(64),
        compatibility_group_digest: "b".repeat(64),
        path: PathBuf::from("/path/that/does/not/exist"),
        exact_hit: true,
    };

    assert_eq!(
        save_cache(&RuntimeClient::disconnected_for_cache_tests(), &cache),
        CacheFinalizationOutcome::Unchanged
    );
}

#[test]
fn cache_input_digest_distinguishes_missing_empty_content_and_environment() {
    let root = tempfile::tempdir().unwrap();
    let inputs = CacheKeyInputs::new(
        vec!["Cargo.lock".to_string()],
        vec!["RUSTUP_TOOLCHAIN".to_string()],
        false,
    )
    .unwrap();
    let mut environment = BTreeMap::from([("RUSTUP_TOOLCHAIN".to_string(), "1.98.0".to_string())]);
    let missing = digest_inputs_at(&inputs, &environment, root.path(), "source-a").unwrap();
    fs::write(root.path().join("Cargo.lock"), []).unwrap();
    let empty = digest_inputs_at(&inputs, &environment, root.path(), "source-a").unwrap();
    fs::write(root.path().join("Cargo.lock"), b"lock").unwrap();
    let content = digest_inputs_at(&inputs, &environment, root.path(), "source-a").unwrap();
    environment.insert("RUSTUP_TOOLCHAIN".to_string(), "1.99.0".to_string());
    let environment_changed =
        digest_inputs_at(&inputs, &environment, root.path(), "source-a").unwrap();

    let source_inputs = CacheKeyInputs::new(vec![], vec![], true).unwrap();
    let source_a = digest_inputs_at(&source_inputs, &environment, root.path(), "source-a").unwrap();
    let source_b = digest_inputs_at(&source_inputs, &environment, root.path(), "source-b").unwrap();

    assert_ne!(missing, empty);
    assert_ne!(empty, content);
    assert_ne!(content, environment_changed);
    assert_ne!(source_a, source_b);
}

#[test]
fn cache_input_hashing_rejects_symlinks_and_directories() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("outside"), b"secret").unwrap();
    symlink("outside", root.path().join("linked")).unwrap();
    fs::create_dir(root.path().join("directory")).unwrap();
    fs::create_dir(root.path().join("outside-directory")).unwrap();
    fs::write(root.path().join("outside-directory/input"), b"secret").unwrap();
    symlink("outside-directory", root.path().join("linked-directory")).unwrap();
    assert!(open_key_file(root.path(), "linked").is_err());
    assert!(open_key_file(root.path(), "directory").is_err());
    assert!(open_key_file(root.path(), "linked-directory/input").is_err());

    let oversized = fs::File::create(root.path().join("oversized")).unwrap();
    oversized.set_len(MAX_CACHE_KEY_FILE_BYTES + 1).unwrap();
    assert!(open_key_file(root.path(), "oversized").is_err());
}

fn populate_cache(root: &Path, reverse: bool, modified_offset: Duration) {
    let files = if reverse {
        [("data.txt", "data"), ("bin/run", "#!/bin/sh\n")]
    } else {
        [("bin/run", "#!/bin/sh\n"), ("data.txt", "data")]
    };
    for (path, contents) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        let mode = if path.ends_with("run") { 0o755 } else { 0o644 };
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + modified_offset))
            .unwrap();
    }
    symlink("bin/run", root.join("run-link")).unwrap();
}
