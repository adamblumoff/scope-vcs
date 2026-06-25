# CLI Init And Clerk Auth Design

## Summary

Scope repo creation moves out of the web app and into a Rust CLI command:
`scope init`. The web app no longer has a create-repository form or setup
flow. The CLI creates the repo through the API, configures the local Git repo,
pushes the committed `HEAD`, and prints the review URL.

This is a destructive pre-alpha change. Shoo auth is removed and replaced with
Clerk across web, API, and CLI. Setup-token regeneration and Git credential
reset flows are removed rather than kept as compatibility paths.

## Goals

- Make `scope init` the only repo creation and first-push entrypoint.
- Replace Shoo with Clerk in one big-bang migration.
- Keep API/domain code as the single source of truth for repo rules,
  lifecycle transitions, identity, tokens, URLs, and review state.
- Keep web and CLI thin: web renders API state; CLI orchestrates local Git
  side effects.
- Show CLI installation from the signed-in empty home state with a copyable
  command that uses the Railway-generated CLI service URL.

## Non-Goals

- No backwards compatibility with the web setup flow.
- No long-lived CLI auth yet.
- No web-based Git credential repair.
- No branded CLI download domain yet.
- No `--api-url` user-facing CLI flag.

## Architecture

The product has three services/surfaces:

- `cli/`: Rust binary named `scope`. Owns local Git detection, prompts, Clerk
  browser login, API calls, Git credential storage, remote configuration,
  first push, and printing the review URL.
- `api/`: Rust Axum service. Owns identity verification, repo creation,
  credential/token issuance, lifecycle state, Git receive, review/publish
  state, and all domain invariants.
- `web/`: TanStack Start app. Owns sign-in UI, repo browsing, review/publish,
  settings, history, and the signed-in empty-state CLI install command.

The API/domain layer remains the source of truth. The CLI must not duplicate
repo creation rules, lifecycle rules, visibility validation, credential
semantics, review URL construction, or publish transitions. It only applies
local machine side effects using API-provided data.

## CLI Flow

`scope init` runs from inside a local Git repository.

Example:

```text
$ scope init

Repository name [scope-vcs]:
Default visibility [Private]:

Working tree has uncommitted changes.
Only committed HEAD will be pushed to Scope.

Sign in to Scope:
https://scopevcs.com/cli/auth/start?...

Waiting for sign-in...
Signed in as adam@example.com

Creating Scope repo adam/scope-vcs...
Configuring git remote scope...
Pushing HEAD to Scope...

Review ready:
https://scopevcs.com/repos/adam/scope-vcs/review
```

Behavior:

- Refuse to run outside a Git repository.
- Refuse to continue if the repo has no committed `HEAD`.
- Allow dirty working trees, but warn that uncommitted changes are not pushed.
- Default repository name to the Git root directory name. Pressing Enter accepts
  the default; typing a value uses that value.
- Default visibility to `Private`. Pressing Enter accepts it; typing `public`
  uses public visibility.
- Support `scope init --name <name>`, `scope init --public`, and
  `scope init --private`.
- Do not expose a user-facing `--api-url` flag. Local development may use an
  internal config or environment override.
- Authenticate with Clerk browser login when there is no valid local token.
- Call the API to create the repo.
- Receive the remote name, remote URL/path, push branch, one-time Git push
  credential, and review URL from the API response.
- Store the Git credential through Git's credential helper.
- Destructively replace the local `scope` remote with the API-provided remote.
- Push current committed `HEAD` to the API-provided branch.
- Print the review URL as a plain clickable URL.
- Do not automatically open the browser after push.

## Clerk Auth

Shoo is removed.

Web:

- Install `@clerk/tanstack-react-start`.
- Add `clerkMiddleware()` to the TanStack Start server entry.
- Wrap the root shell in `<ClerkProvider>`.
- Replace Shoo sign-in/sign-out code with Clerk controls/hooks.
- Remove `/auth/callback`, `/auth/session`, `@shoojs/auth`, and Shoo session
  helpers.

API:

- Replace `ShooIdentity` with a Clerk identity abstraction.
- Verify Clerk session JWTs from `Authorization: Bearer <token>`.
- Use Clerk `user_id` directly as the canonical local `UserAccount.id`.
- Keep local `UserAccount` records as the app profile/cache: id, handle,
  email, email verification, and access.
- Create/update local users from Clerk claims or a lightweight Clerk user lookup
  during authenticated requests.
- Remove Shoo pairwise-sub hashing and verified-email account merge logic.

CLI:

- Start a localhost callback server for browser login.
- Open or print the Clerk login URL.
- Receive a Clerk session token, store it locally, and use it as an API bearer
  token.
- If the token expires, re-run browser login.
- Long-lived auth is deferred.

## API And Domain Changes

Repo creation remains domain-owned and API-mediated.

Changes:

- Keep `POST /v1/repos`, but make the CLI its intended caller.
- The create response must include:
  - repo summary
  - full Git remote URL or enough API-owned data to build it in one place
  - remote name: `scope`
  - push branch
  - one-time Git push credential secret
  - review URL
- Prefer returning `review_url` directly so route construction has one source
  of truth.
- Remove `GET /v1/repos/:owner/:repo/setup`.
- Remove `/v1/repos/:owner/:repo/setup-token`.
- Remove setup-token regeneration domain/db methods.
- Remove Git push credential regeneration endpoint and web wiring.
- Keep first-push token and Git push token generation if the Git receive path
  still needs both during init.
- Keep clone credentials if clone still depends on them; clone credentials are
  separate from init/setup credentials.
- Preserve current first-push lifecycle behavior: first push stages a review;
  web handles publish.
- Reject duplicate repo names, invalid repo names, unauthenticated creation,
  and unsupported visibility in one API/domain path.

## Web Changes

Remove:

- Home repo creation form.
- Navigation from creation to setup.
- Setup route/page.
- Setup progress polling.
- Session-storage setup secret handling.
- Setup command generation UI.
- Git credential reset controls in settings.
- Shoo auth callback/session routes.

Keep/adapt:

- Home repo list.
- Clerk sign-in/sign-out.
- Review, publish, repo detail, settings, and history.
- Primary repo actions should route pending review/staged updates to review.
- `PendingFirstPush` repos should be hidden from the normal home list. In the
  new flow they are incomplete CLI-init artifacts, not web setup tasks.

Signed-in empty state:

- Show a compact CLI-first message.
- Show a copyable install command using a configured Railway-generated CLI
  service URL:

```sh
curl -fsSL <SCOPE_CLI_INSTALL_URL>/install.sh | sh
```

- Tell users to run `scope init` from a local Git repository after install.

`SCOPE_CLI_INSTALL_URL` should be configured for web from the Railway-generated
public domain for the CLI service, for example
`https://scope-cli-production-xxxx.up.railway.app`. The exact generated URL is
deployment state, so web reads it from configuration instead of hard-coding a
branded domain.

## CLI Railway Service

Add a third Railway service for CLI distribution.

- Service root: `cli`.
- Link the service to the same GitHub repository as `scope-api` and
  `scope-web`.
- Configure build/deploy from GitHub with the same Railway conventions as the
  existing services: service root, Railpack config, generated public domain,
  and environment variables managed in Railway.
- Provider: Rust, or a static/minimal server if release artifacts are produced
  elsewhere.
- Public Railway auto-domain enabled.
- Serves `GET /install.sh`.
- Serves platform-specific release artifacts needed by `install.sh`.
- The web app uses this service's generated public URL for
  `SCOPE_CLI_INSTALL_URL`.

The install script must install the `scope` binary and make `scope init`
available on `PATH` for supported platforms. Exact packaging can start narrow
and expand later.

## Error Handling And Recovery

CLI:

- Not in a Git repo: stop before API calls.
- No committed `HEAD`: stop before API calls.
- Dirty working tree: warn, continue.
- Existing `scope` remote: replace destructively and print what changed.
- Duplicate repo name: show the API error and suggest rerunning with `--name`.
- Expired auth: rerun browser login.
- Credential storage failure: stop before pushing.
- Push failure after repo creation: show the Git failure and repo name. The repo
  may remain `PendingFirstPush` and hidden from normal home.

API:

- Repo creation is atomic for metadata.
- Failed first push after successful creation may leave `PendingFirstPush`.
- Return clear JSON errors for duplicate names, invalid names, auth failures,
  and unsupported visibility.

Web:

- Hidden `PendingFirstPush` repos avoid setup dead ends.
- Empty state points users to CLI install and `scope init`.
- Review route continues to handle missing/no review state.

## Testing

API tests:

- Clerk-authenticated repo creation succeeds.
- Unauthenticated repo creation fails.
- Duplicate repo names fail.
- Create response includes one-time Git credential and review URL.
- Setup-token regeneration endpoints are gone.
- Git credential regeneration endpoint is gone.
- Pending-first-push repos are hidden from normal list if filtering is
  server-side.
- First-push receive still stages a review.

CLI tests:

- `scope init` refuses non-Git directories.
- `scope init` refuses repos with no `HEAD`.
- Dirty worktree warns and continues.
- Repo name defaults from folder.
- Visibility defaults to private.
- Public visibility is accepted.
- CLI calls API with expected payload.
- CLI configures/replaces the `scope` remote.
- CLI pushes to the API-provided remote and branch.
- CLI prints the API-provided review URL.

Web tests:

- Home no longer renders create form.
- Signed-in empty state renders the CLI install command and `scope init`.
- Setup route is removed.
- Repo primary action no longer links to setup.

## Rollout

- No compatibility layer.
- Remove Shoo env/config and dependency.
- Add Clerk env/config and documentation.
- Add `cli` workspace/service.
- Add CLI Railway service and configure web `SCOPE_CLI_INSTALL_URL` from the
  generated CLI service public domain.
- Link the CLI Railway service to GitHub and configure it consistently with the
  existing `scope-api` and `scope-web` services.
- Regenerate API TypeScript types after response changes.
- Update README with the CLI-first flow and Railway deployment shape.
