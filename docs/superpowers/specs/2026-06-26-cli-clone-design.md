# CLI Clone Design

## Summary

Add `scope clone` so a signed-in CLI can clone a repository without copying the
long website-generated Git credential command.

The command uses the durable CLI session only for the API call. Git still gets a
separate repo-scoped clone credential minted by the API. This keeps the auth
boundary explicit:

- CLI session token: proves the machine/user to the Scope API.
- Git clone token: proves Git read access to one repository.

## Goals

- Add `scope clone <owner>/<repo> [destination]`.
- Reuse the existing OS-keychain CLI session created by `scope login`.
- Keep Git credentials repo-scoped and separate from the CLI API session.
- Make the website clone command short for permissioned users:
  `scope clone owner/repo`.
- Keep public clone behavior available for public visitors:
  `git clone https://.../git/owner/repo`.
- Make repo-local access rules impossible to miss in tests.
- Split the large CLI binary while adding this command so `scope.rs` does not
  keep absorbing unrelated responsibilities.

## Non-Goals

- No long-lived Git credential that works across all repos.
- No global "clone any repo I can see" token.
- No direct use of the `scope_cli_...` API session as a Git password.
- No change to Clerk browser auth.
- No change to repo permission semantics beyond making them explicit for clone.
- No backward-compatible fallback to the old long website command as the primary
  flow.

## Access Contract

Clone access is resolved for the specific repository being cloned.

| Caller | Command | API token minted? | Git projection |
| --- | --- | --- | --- |
| Public or anonymous user for a public repo | `git clone <remote>` | No | Public projection only |
| Signed-in user who is not an owner/member of that repo | `scope clone owner/repo` | No, fail with forbidden | None |
| Signed-in user who is an owner/member of that repo | `scope clone owner/repo` | Yes, repo-scoped Git clone token | Permissioned owner/member projection |
| Owner/member of a different repo | `scope clone owner/repo` | No, fail with forbidden | None |

The important rule: public access is per repo and only receives the public
projection for that repo. Owners and members of that exact repo receive the
permissioned clone for that repo, including private/member-visible content that
the repo policy exposes to them.

A signed-in nonmember may still use the public `git clone <remote>` command for
a public repo. `scope clone` should not silently downgrade to public clone,
because that would hide permission mistakes and make scripts ambiguous.

## Target Flow

```text
scope login
  -> stores a durable scope_cli_... session in the OS keychain

scope clone adam/scope-vcs
  -> reads the CLI session from the OS keychain
  -> validates it through the API auth boundary
  -> POST /v1/repos/adam/scope-vcs/clone-credential
  -> API checks repo-local owner/member access
  -> API returns a one-repo scope_git_... clone token
  -> CLI installs the token into Scope's Git credential store
  -> CLI runs git clone with proactive basic auth
```

The API endpoint already has the right high-level shape:

```text
POST /v1/repos/{owner}/{repo}/clone-credential
Authorization: Bearer scope_cli_...
```

It must continue to require a Scope user and a repo role. Read access by itself
is not enough for credential minting.

## CLI Shape

Add a new command:

```text
scope clone <owner>/<repo> [destination]
```

Behavior:

- Parse only the canonical `owner/repo` form in the first pass.
- Reject empty owner, empty repo, extra path segments, and full URLs with a clear
  error that shows the expected form.
- Read the stored CLI session using the existing keychain auth helper.
- If no session exists, fail with `not signed in; run scope login`.
- Call the clone credential endpoint using the CLI session bearer token.
- Configure a Scope-owned Git credential helper for the clone remote.
- Approve the one repo-scoped credential with username `scope`.
- Run:

```text
git clone -c http.proactiveAuth=basic <remote-with-scope-username> [destination]
```

The Git credential storage should use the same dedicated Scope credential store
strategy as the current web command, not Git's default global store. The CLI
should own this setup so users do not have to copy multi-command shell snippets.

## CLI Module Cleanup

`cli/src/bin/scope.rs` is already too broad. Add `scope clone` while extracting
the obvious boundaries:

- `cli/src/auth.rs`
  - keychain read/store/delete
  - CLI session validation
  - shared auth error guidance
- `cli/src/git_credentials.rs`
  - remote URL normalization
  - Git credential helper setup
  - `git credential approve`
  - proactive-auth clone command construction
- `cli/src/clone.rs`
  - clone argument parsing
  - clone credential API call
  - orchestration of Git credential setup and `git clone`
- `cli/src/bin/scope.rs`
  - command enum and command dispatch only

This keeps the binary from crossing the 1000-line threshold and gives the clone
workflow a place to grow without tangling login, init, and installer concerns.

## API Shape

Keep using the existing endpoint:

```text
POST /v1/repos/{owner}/{repo}/clone-credential
```

Required behavior:

- Accept dedicated Clerk API tokens and durable CLI sessions through
  `require_scope_user`.
- Resolve credentials to the internal Scope user.
- Load the requested repository by owner/name.
- Build a repo-local principal for that exact repo.
- Require repo read access.
- Require an actual owner/member role on that repo before minting a clone token.
- Store only the clone token hash.
- Return the plaintext clone token once.

The API must not issue a clone token to a user who only has public read access.
The public projection does not need a token.

## Git HTTP Shape

No major Git transport change is needed.

Existing behavior to preserve and test:

- No Git auth on a public repo serves the public projection.
- No Git auth on a private repo challenges without revealing private data.
- A valid repo-scoped Git clone token resolves to the token's Scope user.
- The token is accepted only when that user is still owner/member of the target
  repo.
- The resulting Git response uses the permissioned owner/member projection for
  that repo.

This means a `scope_git_...` token minted for `adam/scope-vcs` must not grant
access to `adam/other-repo`, even when the same user owns both.

## Web Shape

Update the clone UI to make the normal command short.

For permissioned users of the current repo:

```text
scope clone owner/repo
```

For public visitors or signed-in users without a repo role:

```text
git clone https://scopevcs.com/git/owner/repo
```

The UI should label the public command as the public clone. It should not imply
that public clone contains private/member-visible content.

Delete the old long credential command as the primary display path. The website
should not keep a compatibility branch that still recommends manual credential
setup once `scope clone` exists.

## Deletion And Refactor Scope

This PR should remove the old web credential-command flow instead of leaving it
unused.

Delete from the web app:

- `credentialedCloneCommand()`.
- The credentialed clone tests in
  `web/src/features/repo-detail/clone-command.test.ts`.
- The web clone credential loading path:
  - `createCloneCredentialForRequest()` in `web/src/api/repo-detail.ts`.
  - `loadCloneCredential` route/page/dropdown props.
  - clone credential React state, busy state, and error state in
    `RepoCloneDropdown`.
  - `toRepoCloneCredentialView()` in `web/src/api/repo-urls.ts`.
  - Web-only clone credential aliases/views in `web/src/api/types.ts`.
- Shell-specific web command helpers, because the remaining clone commands are
  shell-neutral:
  - `web/src/features/git-command-shell.ts`.
  - `web/src/features/git-command-block.tsx`.

Refactor the web clone UI into a small repo-local command builder:

- `permissionedCloneCommand(owner, repo) -> "scope clone owner/repo"`.
- `publicCloneCommand(remoteUrl) -> "git clone <remoteUrl>"`.
- `RepoCloneDropdown` should become a simple copy surface with no API call on
  open.

Keep in the API:

- `POST /v1/repos/{owner}/{repo}/clone-credential`.
- `RepoCloneCredentialResponse` and token persistence.

Those are not dead code after the web cleanup. They become the CLI credential
minting boundary used by `scope clone`.

Refactor the CLI while adding clone:

- Move keychain/session functions out of `cli/src/bin/scope.rs` into
  `cli/src/auth.rs`.
- Move API request DTOs and authenticated request helpers into
  `cli/src/api.rs`.
- Put Git credential setup in `cli/src/git_credentials.rs` and invoke `git`
  with process arguments, not generated shell snippets.
- Put clone parsing and orchestration in `cli/src/clone.rs`.
- Keep `cli/src/bin/scope.rs` as command parsing and dispatch.

Delete or rewrite tests that only protect the removed implementation:

- Remove tests that assert the website emits a multi-command credential setup
  snippet.
- Replace them with tests for the two visible web commands:
  `scope clone owner/repo` for permissioned users and plain public `git clone`
  for public/nonmember users.
- Add static checks that no production web copy contains
  `git credential approve`, `http.proactiveAuth=basic`, or
  `Preparing clone command`.

## Error Handling

CLI errors:

- Missing keychain session:
  `not signed in; run scope login`
- Invalid repo argument:
  `expected repository as owner/repo`
- API 401:
  delete or ignore the stale cached session and tell the user to run
  `scope login`
- API 403:
  `you are not an owner or member of owner/repo`
- API 404:
  `repo owner/repo not found`
- Missing `git` binary:
  fail clearly before requesting a clone credential if possible
- `git clone` failure:
  return Git's exit status and stderr, preserving the original Git message

Do not fall back from `scope clone` to public clone on 403. A permissioned clone
command should be deterministic.

## Tests

API:

- Clone credential endpoint accepts a valid CLI session.
- Clone credential endpoint rejects signed-in users without a role on that repo.
- Owner/member of repo A cannot mint a clone token for repo B unless they are
  also owner/member of repo B.
- Public unauthenticated Git clone returns only the public projection.
- Permissioned Git clone with a minted token returns private/member-visible
  content for that repo.
- A repo-scoped Git token does not authorize a different repo.

CLI:

- `scope clone owner/repo` parses owner and repo.
- Invalid repository specs fail before API calls.
- Missing session tells the user to run `scope login`.
- 403 from clone credential endpoint does not run `git clone`.
- Successful clone installs the repo-scoped credential and runs Git with
  proactive basic auth.
- Optional destination path is passed to `git clone` without shell interpolation.

Web:

- Permissioned repo detail shows `scope clone owner/repo`.
- Public/nonmember repo detail shows `git clone <remote>`.
- The web clone UI no longer generates the long credential setup command as the
  main copy action.
- Copy labels distinguish permissioned clone from public clone.
- Opening the clone dropdown does not call the clone credential endpoint.

Static checks:

- No production UI copy still recommends the long credential setup command as
  the primary clone path.
- No production web source still references `credentialedCloneCommand`,
  `git credential approve`, `http.proactiveAuth=basic`, or
  `Preparing clone command`.
- `cli/src/bin/scope.rs` remains below 1000 lines after the command is added.

## Commit Plan

1. Add this design document.
2. Delete the old web credential-command path and simplify the web clone UI to
   `scope clone` or public `git clone`.
3. Split CLI auth/session helpers out of `cli/src/bin/scope.rs`.
4. Add Git credential helper utilities owned by the CLI.
5. Add API regression tests for CLI-session clone credentials and repo-specific
   permissioned/public clone behavior.
6. Implement `scope clone <owner>/<repo> [destination]`.
7. Run Rust tests, web tests, CLI-focused tests, static dead-code checks, and
   manual smoke tests for:
   public clone, member clone, owner clone, and nonmember failure.

## Implementation Decisions

- Keep the existing `scope_git_` token prefix in this PR. The important
  contract is repo-scoped storage and authorization, not a new prefix.
- If Git credential approval proves platform-sensitive, hide platform-specific
  command details behind the `git_credentials` module rather than branching in
  command orchestration.
