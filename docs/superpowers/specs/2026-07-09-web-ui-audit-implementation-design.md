# Web UI Audit Implementation Design

## Outcome

Make Scope's web interface feel like a refined source-control product rather
than a status dashboard. Repository content and review work become the primary
surfaces. Navigation, status, safety, responsive behavior, and accessibility
become consistent across routes without adding compatibility layers or broad
UI infrastructure.

This design records the UI audit approved on July 9, 2026. It is intentionally
destructive where replacement is simpler because Scope is pre-alpha.

## Product structure

### Shared application and repository shell

Authenticated and public repository routes use a shared shell with a banner,
skip link, main-content landmark, repository identity, and persistent Code,
Requests, History, and Settings navigation. Settings remains hidden for public
visitors. The active route is visible without relying on color alone.

The shell replaces page-specific Repository, Requests, History, and Settings
buttons as well as redundant linked repository IDs and repeated count badges.

### Code view

The repository index becomes a Code view. Its file tree is interactive and
drives a source pane. The selected file is represented in the URL so reload,
back/forward navigation, and sharing preserve context.

The existing repository read boundary owns file content. The web layer only
parses route input, loads the selected file, and renders it. README files receive
a readable prose treatment when safe; other text files use the existing code
and diff typography. Binary or unavailable content receives a clear empty state.

The view includes concise latest-commit context when the existing read model can
provide it. It does not add editing, branch management, or speculative source
hosting behavior.

### Request review

Request detail is reorganized around Overview, Changes, and Activity. Changes
uses the existing file tree and diff viewer. The selected surface and file are
URL state. Overview contains the request summary, collaboration, and only the
workflow actions currently allowed by domain permissions. Activity contains the
timeline.

Workflow transitions remain domain-owned. The UI does not infer additional
states or permissions. Destructive request deletion uses the shared confirmation
primitive.

### Safety and identity

Request deletion, repository member removal, pending invite revocation, and CLI
session revocation require an AlertDialog confirmation that names the affected
object and action.

Human identity is displayed whenever the read model already provides a handle
or email. Internal IDs remain secondary copyable details when they are needed
for debugging or current API input. Adding request editors continues to use the
current input contract until the API exposes a safe lookup surface; the UI makes
that limitation explicit instead of presenting a raw ID as a friendly workflow.

### Responsive and accessible behavior

Desktop density remains compact. Coarse pointers receive at least 44 px hit
targets. Important page titles wrap instead of truncating. Mobile file rows show
explicit Status and Visibility labels. Folder controls expose expanded state and
the tree uses list/tree semantics.

The app shell provides a skip link and correct banner/main landmarks. Async
errors are announced next to their action, loading labels use an ellipsis, and
auth loading has a structural fallback. Existing layout-property animations and
`transition-all` are removed. Remaining motion is short, compositor-only, and
disabled for reduced-motion users.

## UI audit harness

Add `pnpm ui:audit` under `web/` with audit code outside `src/`. It writes the
required `report.json` and screenshots to `UI_AUDIT_DIR`. The initial route
matrix covers public repository Code, Requests, and History at desktop and
390 px mobile widths. Authenticated routes are included only when a deterministic
local session fixture can be provided without production-only bypass code.

## Architecture

- Domain and persistence layers continue to own visibility, request state,
  permissions, and file-read authorization.
- Server functions translate route input and call existing API helpers.
- Shared route UI is split into behavior-owned components: app shell,
  repository navigation, source viewer, request review navigation, and
  destructive confirmation.
- The 977-line request detail module is reduced by moving distinct surfaces into
  separate feature modules; state orchestration remains in one place.
- Replacement should remove old page-specific navigation and duplicated markup
  rather than preserve both paths.

## Error handling

- File reads distinguish unavailable, binary, and service errors.
- Diff and source panes retain their surrounding navigation when loading fails.
- Mutation errors stay adjacent to the initiating control and are announced.
- Auth loading and failure do not leave an unexplained blank pane.

## Testing and verification

- Add focused tests only for new URL parsing, source-selection behavior, and
  meaningful confirmation or request-review state transitions.
- Do not snapshot entire pages or duplicate primitive-library tests.
- Run `pnpm check`, `pnpm test`, focused Rust/API tests if read contracts change,
  and `pnpm ui:audit` against the remote Linux local-dev stack.
- Inspect Code, Requests, History, Settings, request review, dialogs, and auth in
  the in-app browser at desktop and mobile widths.

## Completion criteria

- Code, Requests, History, and Settings are discoverable from repository routes.
- Repository files and request changes can be opened and reviewed.
- Selected source and review context survives reload and browser navigation.
- Audited routes do not overflow horizontally at 390 px.
- Coarse-pointer controls are at least 44 px.
- Destructive actions require explicit confirmation.
- Public routes remain usable while auth client loading is slow or unavailable.
- Web checks, focused regressions, and the UI audit pass.
