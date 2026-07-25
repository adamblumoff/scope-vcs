use super::{
    SeedGitCommit, apply_seed_commits, canonical_request_ref, seed_git, seed_git_head,
    store_seed_bundle,
};
use crate::{
    domain::store::{SourceBlob, StoredRepository},
    error::ApiError,
    object_store::ObjectStore,
};
use std::path::Path;

const REQUEST_NAME: &str = "bounded-retry-timing";
const BUNDLE_LABEL: &str = "req_demo_ready";
const RETRY_NAMED_CAP: &str = concat!(
    "const MAX_RETRY_DELAY_MS = 2_000\n\n",
    "/** Returns the retry delay in milliseconds. */\n",
    "export function retryDelay(attempt: number) {\n",
    "  return Math.min(attempt * 250, MAX_RETRY_DELAY_MS)\n",
    "}\n",
);
const RETRY_WITH_JITTER: &str = concat!(
    "const MAX_RETRY_DELAY_MS = 2_000\n",
    "const MAX_JITTER_MS = 125\n\n",
    "/** Returns the retry delay in milliseconds with bounded positive jitter. */\n",
    "export function retryDelay(attempt: number, random = Math.random) {\n",
    "  const base = Math.min(attempt * 250, MAX_RETRY_DELAY_MS)\n",
    "  return base + Math.floor(random() * MAX_JITTER_MS)\n",
    "}\n",
);
const RETRY_TESTS: &str = concat!(
    "import { strict as assert } from 'node:assert'\n",
    "import { retryDelay } from '../src/retry'\n\n",
    "assert.equal(retryDelay(20, () => 0), 2_000)\n",
    "assert.equal(retryDelay(20, () => 0.999), 2_124)\n",
);
const RETRY_GUIDE: &str = concat!(
    "# Retry behavior\n\n",
    "Remote operations use bounded linear backoff with a two-second cap. ",
    "A small positive jitter keeps simultaneous clients from retrying together.\n",
);
const RETRY_FINAL: &str = concat!(
    "export const RETRY_POLICY = { maxDelayMs: 2_000, maxJitterMs: 125 } as const\n\n",
    "/** Returns the retry delay in milliseconds with bounded positive jitter. */\n",
    "export function retryDelay(attempt: number, random = Math.random) {\n",
    "  const base = Math.min(attempt * 250, RETRY_POLICY.maxDelayMs)\n",
    "  return base + Math.floor(random() * RETRY_POLICY.maxJitterMs)\n",
    "}\n",
);

pub(super) struct SeedRequestRevision {
    pub head_oid: String,
    pub snapshot: SourceBlob,
    pub note: &'static str,
}

struct RevisionSpec<'a> {
    commit: SeedGitCommit<'a>,
    note: &'static str,
}

pub(super) fn seed_bounded_retry_revisions(
    object_store: &dyn ObjectStore,
    repo: &StoredRepository,
    repo_path: &Path,
    initial_head_oid: &str,
    main_oid: &str,
) -> Result<Vec<SeedRequestRevision>, ApiError> {
    let specs = [
        RevisionSpec {
            commit: SeedGitCommit {
                files: &[("src/retry.ts", RETRY_NAMED_CAP)],
                message: "Name the retry cap",
            },
            note: "Extract the retry cap and document milliseconds.",
        },
        RevisionSpec {
            commit: SeedGitCommit {
                files: &[("src/retry.ts", RETRY_WITH_JITTER)],
                message: "Add bounded retry jitter",
            },
            note: "Add bounded positive jitter after maintainer feedback.",
        },
        RevisionSpec {
            commit: SeedGitCommit {
                files: &[("tests/retry.test.ts", RETRY_TESTS)],
                message: "Test retry jitter bounds",
            },
            note: "Cover the retry cap and jitter range with deterministic tests.",
        },
        RevisionSpec {
            commit: SeedGitCommit {
                files: &[
                    ("src/retry.ts", RETRY_FINAL),
                    ("docs/retries.md", RETRY_GUIDE),
                ],
                message: "Document the retry policy",
            },
            note: "Publish the retry policy and add the contributor guide.",
        },
    ];

    let mut previous_head_oid = initial_head_oid.to_string();
    let mut revisions = Vec::with_capacity(specs.len());
    for (index, spec) in specs.into_iter().enumerate() {
        let head_oid = seed_revision(repo_path, spec.commit, &previous_head_oid, main_oid)?;
        let snapshot = store_seed_bundle(
            object_store,
            repo,
            repo_path,
            &format!("{BUNDLE_LABEL}_{}", index + 1),
            &[&canonical_request_ref(REQUEST_NAME)],
            &head_oid,
        )?;
        previous_head_oid = head_oid.clone();
        revisions.push(SeedRequestRevision {
            head_oid,
            snapshot,
            note: spec.note,
        });
    }
    Ok(revisions)
}

fn seed_revision(
    repo_path: &Path,
    commit: SeedGitCommit<'_>,
    previous_head_oid: &str,
    main_oid: &str,
) -> Result<String, ApiError> {
    seed_git(
        Some(repo_path),
        &["reset", "--hard", previous_head_oid],
        "restoring seeded request revision",
    )?;
    apply_seed_commits(repo_path, &[commit])?;
    let head_oid = seed_git_head(repo_path)?;
    let request_ref = canonical_request_ref(REQUEST_NAME);
    seed_git(
        Some(repo_path),
        &["update-ref", &request_ref, &head_oid],
        "advancing seeded request ref",
    )?;
    seed_git(
        Some(repo_path),
        &["reset", "--hard", main_oid],
        "restoring seeded main branch",
    )?;
    Ok(head_oid)
}
