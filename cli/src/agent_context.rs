use anyhow::{Context, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const RULES_RELATIVE_PATH: &str = ".scope/RULES.md";
const CODEX_FILE: &str = "AGENTS.md";
const CODEX_OVERRIDE_FILE: &str = "AGENTS.override.md";
const CLAUDE_FILE: &str = "CLAUDE.md";
const CLAUDE_LOCAL_FILE: &str = "CLAUDE.local.md";
const START_MARKER: &str = "<!-- scope:rules:start -->";
const END_MARKER: &str = "<!-- scope:rules:end -->";

const CODEX_BLOCK: &str = "<!-- scope:rules:start -->\n## Scope contribution rules\n\nRead and follow `.scope/RULES.md` before\nmaking or submitting changes.\n<!-- scope:rules:end -->";
const CLAUDE_BLOCK: &str = "<!-- scope:rules:start -->\n@.scope/RULES.md\n<!-- scope:rules:end -->";

#[derive(Debug, Default, Eq, PartialEq)]
pub struct SyncResult {
    pub changed_paths: Vec<PathBuf>,
}

pub fn sync_repo_rules(git_root: &Path) -> anyhow::Result<SyncResult> {
    let mut changed_paths = Vec::new();
    let rules_path = git_root.join(RULES_RELATIVE_PATH);
    reject_symlink(
        rules_path
            .parent()
            .expect("canonical rules path has a parent"),
    )?;
    reject_symlink(&rules_path)?;
    if !rules_path.exists() {
        fs::create_dir_all(
            rules_path
                .parent()
                .expect("canonical rules path has a parent"),
        )
        .with_context(|| format!("create {}", rules_path.display()))?;
        fs::write(&rules_path, []).with_context(|| format!("create {}", rules_path.display()))?;
        changed_paths.push(PathBuf::from(RULES_RELATIVE_PATH));
    } else if !rules_path.is_file() {
        bail!("{} must be a file", rules_path.display());
    }

    for adapter in detected_adapters(git_root) {
        let path = git_root.join(adapter.path);
        reject_symlink(&path)?;
        let current = match fs::read_to_string(&path) {
            Ok(current) => current,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let desired = managed_content(&current, adapter.block)
            .with_context(|| format!("update {}", path.display()))?;
        if desired != current {
            fs::write(&path, desired).with_context(|| format!("write {}", path.display()))?;
            changed_paths.push(PathBuf::from(adapter.path));
        }
    }

    Ok(SyncResult { changed_paths })
}

pub fn ensure_repo_rules_ready_for_push(git_root: &Path, head_oid: &str) -> anyhow::Result<()> {
    let result = (|| {
        ensure_worktree_is_synced(git_root)?;
        ensure_head_file(git_root, head_oid, RULES_RELATIVE_PATH, None)?;
        for adapter in detected_head_adapters(git_root, head_oid)? {
            ensure_head_file(git_root, head_oid, adapter.path, Some(adapter.block))?;
        }
        Ok(())
    })();

    result.map_err(|error: anyhow::Error| {
        error
            .context("Run `scope rules sync`, commit the generated files, then retry `scope push`.")
    })
}

fn ensure_worktree_is_synced(git_root: &Path) -> anyhow::Result<()> {
    let rules_path = git_root.join(RULES_RELATIVE_PATH);
    reject_symlink(
        rules_path
            .parent()
            .expect("canonical rules path has a parent"),
    )?;
    reject_symlink(&rules_path)?;
    if !rules_path.is_file() {
        bail!("{} is required", rules_path.display());
    }
    for adapter in detected_adapters(git_root) {
        let path = git_root.join(adapter.path);
        reject_symlink(&path)?;
        let current =
            fs::read_to_string(&path).with_context(|| format!("{} is required", path.display()))?;
        if managed_content(&current, adapter.block)? != current {
            bail!(
                "{} does not contain the current Scope rules link",
                path.display()
            );
        }
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("{} must not be a symlink", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}

fn ensure_head_file(
    git_root: &Path,
    head_oid: &str,
    relative_path: &str,
    managed_block: Option<&str>,
) -> anyhow::Result<()> {
    let revision = format!("{head_oid}:{relative_path}");
    let output = Command::new("git")
        .current_dir(git_root)
        .args(["show", &revision])
        .output()
        .with_context(|| format!("read {relative_path} from pushed commit"))?;
    if !output.status.success() {
        bail!("{relative_path} is not committed in the pushed tree");
    }
    if let Some(block) = managed_block {
        let current = String::from_utf8(output.stdout)
            .with_context(|| format!("committed {relative_path} is not UTF-8"))?;
        if managed_content(&current, block)? != current {
            bail!("committed {relative_path} does not contain the current Scope rules link");
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Adapter {
    path: &'static str,
    block: &'static str,
}

fn detected_adapters(git_root: &Path) -> Vec<Adapter> {
    let mut paths = git_visible_paths(git_root);
    for path in [
        CODEX_OVERRIDE_FILE,
        CODEX_FILE,
        CLAUDE_FILE,
        CLAUDE_LOCAL_FILE,
        ".mcp.json",
    ] {
        if git_root.join(path).is_file() && !paths.iter().any(|visible| visible == path) {
            paths.push(path.to_owned());
        }
    }
    for path in [".codex", ".agents", ".claude"] {
        if git_root.join(path).is_dir() {
            paths.push(path.to_owned());
        }
    }
    adapters_for_paths(&paths)
}

fn adapters_for_paths(paths: &[String]) -> Vec<Adapter> {
    let has_path = |expected: &str| paths.iter().any(|path| path == expected);
    let has_directory = |directory: &str| {
        let prefix = format!("{directory}/");
        paths
            .iter()
            .any(|path| path == directory || path.starts_with(&prefix))
    };
    let has_codex_context = paths
        .iter()
        .any(|path| matches!(path_basename(path), "AGENTS.md" | "AGENTS.override.md"));
    let has_claude_context = paths
        .iter()
        .any(|path| matches!(path_basename(path), "CLAUDE.md" | "CLAUDE.local.md"));

    let mut adapters = Vec::new();
    if has_path(CODEX_OVERRIDE_FILE) {
        adapters.push(Adapter {
            path: CODEX_OVERRIDE_FILE,
            block: CODEX_BLOCK,
        });
    } else if has_directory(".codex")
        || has_directory(".agents")
        || has_path(CODEX_FILE)
        || has_codex_context
    {
        adapters.push(Adapter {
            path: CODEX_FILE,
            block: CODEX_BLOCK,
        });
    }
    if has_directory(".claude")
        || has_path(CLAUDE_FILE)
        || has_path(CLAUDE_LOCAL_FILE)
        || has_path(".mcp.json")
        || has_claude_context
    {
        adapters.push(Adapter {
            path: CLAUDE_FILE,
            block: CLAUDE_BLOCK,
        });
    }
    adapters
}

fn detected_head_adapters(git_root: &Path, head_oid: &str) -> anyhow::Result<Vec<Adapter>> {
    Ok(adapters_for_paths(&git_tree_paths(git_root, head_oid)?))
}

fn git_tree_paths(git_root: &Path, head_oid: &str) -> anyhow::Result<Vec<String>> {
    let output = Command::new("git")
        .current_dir(git_root)
        .args(["ls-tree", "-r", "-z", "--name-only", head_oid])
        .output()
        .context("inspect pushed tree for agent context")?;
    if !output.status.success() {
        bail!("could not inspect pushed tree for agent context");
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

fn git_visible_paths(git_root: &Path) -> Vec<String> {
    let Ok(output) = Command::new("git")
        .current_dir(git_root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect()
}

fn path_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn managed_content(current: &str, block: &str) -> anyhow::Result<String> {
    let starts = current.match_indices(START_MARKER).collect::<Vec<_>>();
    let ends = current.match_indices(END_MARKER).collect::<Vec<_>>();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => {
            if current.is_empty() {
                Ok(format!("{block}\n"))
            } else {
                Ok(format!(
                    "{}{}{}\n",
                    current,
                    if current.ends_with('\n') {
                        "\n"
                    } else {
                        "\n\n"
                    },
                    block
                ))
            }
        }
        ([(start, _)], [(end, _)]) if start < end => {
            let suffix_start = end + END_MARKER.len();
            Ok(format!(
                "{}{}{}",
                &current[..*start],
                block,
                &current[suffix_start..]
            ))
        }
        _ => bail!("Scope rules markers are missing, duplicated, or out of order"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn no_agent_signal_creates_only_empty_rules() {
        let repo = TestDir::new("rules-no-agent");

        let result = sync_repo_rules(repo.path()).unwrap();

        assert_eq!(result.changed_paths, [PathBuf::from(RULES_RELATIVE_PATH)]);
        assert_eq!(
            fs::read(repo.path().join(RULES_RELATIVE_PATH)).unwrap(),
            b""
        );
        assert!(!repo.path().join(CODEX_FILE).exists());
        assert!(!repo.path().join(CLAUDE_FILE).exists());
    }

    #[test]
    fn dot_directories_signal_repo_level_adapters_and_sync_is_idempotent() {
        let repo = TestDir::new("rules-agent-signals");
        fs::create_dir(repo.path().join(".codex")).unwrap();
        fs::create_dir(repo.path().join(".claude")).unwrap();

        let first = sync_repo_rules(repo.path()).unwrap();
        let second = sync_repo_rules(repo.path()).unwrap();

        assert_eq!(first.changed_paths.len(), 3);
        assert!(second.changed_paths.is_empty());
        assert!(
            fs::read_to_string(repo.path().join(CODEX_FILE))
                .unwrap()
                .contains(CODEX_BLOCK)
        );
        assert!(
            fs::read_to_string(repo.path().join(CLAUDE_FILE))
                .unwrap()
                .contains(CLAUDE_BLOCK)
        );
        assert!(!repo.path().join(".codex/AGENTS.md").exists());
        assert!(!repo.path().join(".claude/CLAUDE.md").exists());
    }

    #[test]
    fn existing_adapter_content_is_preserved_around_managed_block() {
        let repo = TestDir::new("rules-existing-adapter");
        fs::write(repo.path().join(CODEX_FILE), "project guidance\n").unwrap();

        sync_repo_rules(repo.path()).unwrap();

        let content = fs::read_to_string(repo.path().join(CODEX_FILE)).unwrap();
        assert!(content.starts_with("project guidance\n\n"));
        assert!(content.ends_with(&format!("{CODEX_BLOCK}\n")));
    }

    #[test]
    fn codex_override_receives_the_link_instead_of_inactive_agents_file() {
        let repo = TestDir::new("rules-codex-override");
        fs::write(repo.path().join(CODEX_FILE), "ordinary guidance\n").unwrap();
        fs::write(repo.path().join(CODEX_OVERRIDE_FILE), "active override\n").unwrap();

        let result = sync_repo_rules(repo.path()).unwrap();

        assert_eq!(
            fs::read_to_string(repo.path().join(CODEX_FILE)).unwrap(),
            "ordinary guidance\n"
        );
        assert!(
            fs::read_to_string(repo.path().join(CODEX_OVERRIDE_FILE))
                .unwrap()
                .contains(CODEX_BLOCK)
        );
        assert!(
            result
                .changed_paths
                .contains(&PathBuf::from(CODEX_OVERRIDE_FILE))
        );
    }

    #[test]
    fn nested_and_local_native_contexts_trigger_root_adapters() {
        let repo = TestDir::git_repo("rules-nested-context", "main");
        fs::create_dir(repo.path().join("src")).unwrap();
        fs::write(repo.path().join("src/AGENTS.override.md"), "nested\n").unwrap();
        fs::write(repo.path().join(CLAUDE_LOCAL_FILE), "local\n").unwrap();
        repo.run_git(["add", "src/AGENTS.override.md", CLAUDE_LOCAL_FILE]);

        sync_repo_rules(repo.path()).unwrap();

        assert!(
            fs::read_to_string(repo.path().join(CODEX_FILE))
                .unwrap()
                .contains(CODEX_BLOCK)
        );
        assert!(
            fs::read_to_string(repo.path().join(CLAUDE_FILE))
                .unwrap()
                .contains(CLAUDE_BLOCK)
        );
    }

    #[test]
    fn malformed_managed_markers_are_not_overwritten() {
        let repo = TestDir::new("rules-malformed-adapter");
        fs::write(repo.path().join(CODEX_FILE), START_MARKER).unwrap();

        let error = sync_repo_rules(repo.path()).unwrap_err();

        assert!(error.to_string().contains("update"));
    }

    #[test]
    fn push_preflight_requires_synced_files_in_the_committed_tree() {
        let repo = TestDir::git_repo("rules-push-preflight", "main");
        fs::create_dir(repo.path().join(".codex")).unwrap();
        sync_repo_rules(repo.path()).unwrap();

        let uncommitted_error = ensure_repo_rules_ready_for_push(repo.path(), "HEAD").unwrap_err();
        assert!(
            uncommitted_error
                .to_string()
                .contains("Run `scope rules sync`")
        );

        repo.run_git(["add", ".scope/RULES.md", "AGENTS.md"]);
        repo.run_git([
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "-m",
            "add rules context",
        ]);
        let head = String::from_utf8(repo.run_git(["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();

        ensure_repo_rules_ready_for_push(repo.path(), &head).unwrap();
    }

    #[test]
    fn push_preflight_uses_committed_agent_signals() {
        let repo = TestDir::git_repo("rules-committed-signal", "main");
        fs::create_dir_all(repo.path().join(".scope")).unwrap();
        fs::write(repo.path().join(RULES_RELATIVE_PATH), []).unwrap();
        fs::create_dir(repo.path().join(".codex")).unwrap();
        fs::write(repo.path().join(".codex/config.toml"), "model = 'scope'\n").unwrap();
        repo.run_git(["add", RULES_RELATIVE_PATH, ".codex/config.toml"]);
        repo.run_git([
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "-m",
            "commit unsynced signal",
        ]);
        repo.run_git(["rm", ".codex/config.toml"]);

        let error = ensure_repo_rules_ready_for_push(repo.path(), "HEAD").unwrap_err();

        assert!(format!("{error:#}").contains("AGENTS.md is not committed"));
    }

    #[test]
    fn push_preflight_uses_committed_codex_override() {
        let repo = TestDir::git_repo("rules-committed-override", "main");
        fs::create_dir_all(repo.path().join(".scope")).unwrap();
        fs::write(repo.path().join(RULES_RELATIVE_PATH), []).unwrap();
        fs::write(repo.path().join(CODEX_FILE), format!("{CODEX_BLOCK}\n")).unwrap();
        fs::write(repo.path().join(CODEX_OVERRIDE_FILE), "active override\n").unwrap();
        repo.run_git(["add", RULES_RELATIVE_PATH, CODEX_FILE, CODEX_OVERRIDE_FILE]);
        repo.run_git([
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "-m",
            "commit unsynced override",
        ]);
        repo.run_git(["rm", CODEX_OVERRIDE_FILE]);

        let error = ensure_repo_rules_ready_for_push(repo.path(), "HEAD").unwrap_err();

        assert!(format!("{error:#}").contains("AGENTS.override.md does not contain"));
    }

    #[cfg(unix)]
    #[test]
    fn push_preflight_rejects_symlinked_scope_directory() {
        use std::os::unix::fs::symlink;

        let repo = TestDir::git_repo("rules-symlinked-parent", "main");
        fs::create_dir(repo.path().join("rules-target")).unwrap();
        fs::write(repo.path().join("rules-target/RULES.md"), []).unwrap();
        symlink("rules-target", repo.path().join(".scope")).unwrap();

        let error = ensure_repo_rules_ready_for_push(repo.path(), "HEAD").unwrap_err();

        assert!(format!("{error:#}").contains("must not be a symlink"));
    }
}
