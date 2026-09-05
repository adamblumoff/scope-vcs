# Retired local Git storage cutover

Migration `m0035_retired_git_storage_cutover` forces a maintenance deployment.
Stop all API and worker processes before applying it. For every retained API
filesystem, run the new maintenance binary as the API filesystem owner, with
`SCOPE_DATA_DIR` set to that exact absolute private data root and `DATABASE_URL`
pointing to the deployment database:

```sh
scope-maintenance apply
scope-maintenance scrub-retired-git-storage
```

The scrub holds the database writer fence and an exclusive local file lock. It
rejects symlinks, filesystem boundaries, unrecognized names, and non-private data
roots before deleting anything. Each deletion is logged; successful command
output includes `retiredGitPathsDeleted` and `complete: true`. Run the command
again after interruption. Completion is recorded atomically only after deletion
has been synced to disk.

The deployment's `run_api_maintenance` helper creates a temporary directory on
the CI host; it cannot scrub a retained API filesystem. Do not use that helper
for this command. A replacement container with a fresh ephemeral data directory
initializes its own completion marker. A retained directory with retired data
refuses API startup until maintenance finishes. Complete every retained root
before reopening writers. An ambiguous path requires operator investigation;
renaming it to bypass the gate is not a completed cutover.

The scrub removes pre-incarnation request-ref repositories and locks,
receive-pack staging, and retired `git-repos`/`git-staged` repositories. It leaves
current incarnation paths, managed Git caches, durable segments, and object-store
snapshots intact. Old objects are never imported or served.

On September 5, 2026, read-only Railway service configuration and active
deployment metadata showed no API volume mounts in production or staging
(`volumeMounts: []`). Those deployments use the fresh-container path. Recheck
mounts when deploying; this observation does not exempt retained local roots.
