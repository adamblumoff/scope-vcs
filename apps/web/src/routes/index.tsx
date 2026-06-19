import {
  Alert,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { authCookieName, createScopeShooAuth } from '@/lib/auth'
import { cn } from '@/lib/utils'
import { Link, createFileRoute } from '@tanstack/react-router'
import type { ErrorComponentProps } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  AlertCircle,
  GitBranch,
  LoaderCircle,
  LogIn,
  LogOut,
  Moon,
  Settings,
  Sun,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useMemo, useState } from 'react'

type PrincipalId = string
type RepoRole = 'Reader' | 'Writer' | 'Maintainer' | 'Owner'

type ProjectedChange = {
  path: string
}

type ProjectedCommit = {
  projected_id: string
  logical_commit_id: string
  parent_projected_id: string | null
  author: string | null
  message: string
  synthetic: boolean
  changes: ProjectedChange[]
}

type Projection = {
  repo_id: string
  principal_id: string
  commits: ProjectedCommit[]
}

type GitProjection = {
  principal_id: string
  blobs: Array<{
    path: string
    oid: string
  }>
  head_oid: string | null
}

type FileVisibility = 'Public' | 'Private'

type RepoFile = {
  path: string
  oid: string
  tracked: boolean
  visibility: FileVisibility
}

type LoadState<T> = {
  data: T | null
  error: string | null
  loading: boolean
  status: number | null
}

type GitBoundaryState = {
  state: 'explicit' | 'unexpected' | 'error'
  detail: string
}

type WorkspaceData = {
  files: LoadState<RepoFile[]>
  gitBoundary: GitBoundaryState
  gitProjection: LoadState<GitProjection>
  projection: LoadState<Projection>
  session: LoadState<SessionResponse>
}

type SessionResponse = {
  identity: {
    pairwise_sub: string
    email: string | null
    email_verified: boolean
  } | null
  repo: {
    id: string
    role: RepoRole | null
  }
  principal_id: string
  capabilities: {
    read: boolean
    write: boolean
  }
}

type ThemeMode = 'dark' | 'light'

const repoOwner = 'adamblumoff'
const repoName = 'scope-vcs'
const repoId = `${repoOwner}/${repoName}`
const localApiBase = 'http://localhost:8080'

type SetFileVisibilityInput = {
  path: string
  visibility: FileVisibility
}

type PendingVisibilityChange = {
  file: RepoFile
  visibility: FileVisibility
}

const loadWorkspaceForRequest = createServerFn({ method: 'GET' }).handler(
  async () => {
    const idToken = await readRequestAuthToken()
    const workspace = await loadWorkspace(idToken)

    if (idToken && workspace.session.status === 401) {
      const { deleteCookie } = await import('@tanstack/react-start/server')
      deleteCookie(authCookieName, { path: '/' })
      return loadWorkspace()
    }

    return workspace
  },
)

const setFileVisibilityForRequest = createServerFn({ method: 'POST' })
  .validator(parseSetFileVisibilityInput)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()

    if (!idToken) {
      throw new Error('Sign in as the repo owner to change file visibility.')
    }

    const api = getApiMutationConnection()
    const response = await fetch(
      `${api}/v1/repos/${repoOwner}/${repoName}/files/visibility`,
      {
        body: JSON.stringify(data),
        headers: {
          ...authHeaders(idToken),
          'content-type': 'application/json',
        },
        method: 'PATCH',
      },
    )
    const payload = await response.json().catch(() => null)

    if (!response.ok) {
      throw new Error(payload?.error ?? `request failed: ${response.status}`)
    }

    return payload as RepoFile
  })

export const Route = createFileRoute('/')({
  loader: () => loadWorkspaceForRequest(),
  errorComponent: WorkspaceError,
  component: ScopeWorkspace,
})

function ScopeWorkspace() {
  const workspace = Route.useLoaderData()
  const [files, setFiles] = useState(workspace.files)
  const [fileUpdateError, setFileUpdateError] = useState<string | null>(null)
  const [pendingFile, setPendingFile] = useState<string | null>(null)
  const [pendingVisibility, setPendingVisibility] =
    useState<PendingVisibilityChange | null>(null)
  const [theme, setTheme] = useState<ThemeMode>('dark')

  const session = workspace.session.data
  const projection = workspace.projection.data
  const repoFiles = files.data ?? []
  const commits = useMemo(
    () => projection?.commits.slice().reverse() ?? [],
    [projection],
  )
  const role = session?.repo.role ?? null
  const roleLabel = role ?? 'Public'
  const principal = session?.principal_id ?? 'public'
  const canWrite = session?.capabilities.write ?? false
  const canManageFiles = role === 'Owner'
  const signedIn = Boolean(session?.identity)

  function toggleTheme() {
    const nextTheme = theme === 'dark' ? 'light' : 'dark'
    setTheme(nextTheme)
    applyTheme(nextTheme)
  }

  function requestFileVisibility(file: RepoFile, visibility: FileVisibility) {
    if (file.visibility === visibility) {
      return
    }

    setPendingVisibility({ file, visibility })
  }

  async function confirmFileVisibility() {
    if (!pendingVisibility) {
      return
    }

    const { file, visibility } = pendingVisibility
    setPendingVisibility(null)

    setPendingFile(file.path)
    setFileUpdateError(null)

    try {
      const updated = await setFileVisibilityForRequest({
        data: { path: file.path, visibility },
      })
      setFiles((current) => ({
        ...current,
        data:
          current.data?.map((candidate) =>
            candidate.path === updated.path ? updated : candidate,
          ) ?? [updated],
      }))
    } catch (error) {
      setFileUpdateError(
        error instanceof Error ? error.message : 'visibility update failed',
      )
    } finally {
      setPendingFile(null)
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <header className="border-b border-border bg-background">
        <div className="mx-auto flex min-h-16 max-w-[1180px] flex-wrap items-center justify-between gap-3 px-4 py-3 sm:px-6">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-md border border-border">
              <GitBranch className="size-4" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold leading-5">Scope</div>
              <div className="truncate font-mono text-xs leading-4 text-muted-foreground">
                {repoId}
              </div>
            </div>
          </div>

          <div className="flex min-w-0 items-center gap-2">
            <AuthControls
              session={session}
              signedIn={signedIn}
            />
            <ThemeToggle theme={theme} toggleTheme={toggleTheme} />
          </div>
        </div>
      </header>

      <section className="mx-auto max-w-[1180px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-5 border-b border-border pb-6 lg:flex-row lg:items-end lg:justify-between">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <StatusBadge tone={workspace.session.error ? 'bad' : 'good'}>
                {roleLabel}
              </StatusBadge>
              <StatusBadge tone={session?.capabilities.read ? 'good' : 'neutral'}>
                {principal === 'public' ? 'Public view' : 'Verified session'}
              </StatusBadge>
            </div>
            <h1 className="truncate font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              {repoId}
            </h1>
          </div>

          <RepoActions
            role={role}
          />
        </div>

        <WorkspaceAlerts
          files={files}
          gitBoundary={workspace.gitBoundary}
          gitProjection={workspace.gitProjection}
          projection={workspace.projection}
          session={workspace.session}
        />
        {fileUpdateError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Visibility update failed</AlertTitle>
            <AlertDescription>{fileUpdateError}</AlertDescription>
          </Alert>
        )}

        <MetricStrip
          blobs={repoFiles.length}
          commits={projection?.commits.length ?? 0}
          paths={repoFiles.length}
          writeEnabled={canWrite}
        />

        <div className="grid gap-8 pt-8 lg:grid-cols-[minmax(0,1fr)_340px]">
          <section className="min-w-0">
            <SectionTitle
              action={<Badge variant="outline">{repoFiles.length} files</Badge>}
              title="Git Files"
            />
            <RepoFileTable
              canManageFiles={canManageFiles}
              files={files}
              onSetVisibility={requestFileVisibility}
              pendingFile={pendingFile}
            />
          </section>

          <aside className="min-w-0 space-y-8">
            <SessionPanel
              canWrite={canWrite}
              principal={principal}
              session={session}
              signedIn={signedIn}
            />
          </aside>
        </div>

        <section className="pt-9">
          <SectionTitle
            action={<Badge variant="outline">{commits.length} commits</Badge>}
            title="Visible History"
          />
          <CommitList commits={commits} />
        </section>
      </section>

      <VisibilityConfirmDialog
        confirmation={pendingVisibility}
        onCancel={() => setPendingVisibility(null)}
        onConfirm={() => void confirmFileVisibility()}
      />
    </main>
  )
}

function ThemeToggle({
  theme,
  toggleTheme,
}: {
  theme: ThemeMode
  toggleTheme: () => void
}) {
  const nextTheme = theme === 'dark' ? 'Light' : 'Dark'

  return (
    <Button
      aria-label={`Switch to ${nextTheme} mode`}
      onClick={toggleTheme}
      size="icon-sm"
      title={`Switch to ${nextTheme} mode`}
      variant="secondary"
    >
      {theme === 'dark' ? (
        <Sun className="size-3.5" />
      ) : (
        <Moon className="size-3.5" />
      )}
    </Button>
  )
}

function AuthControls({
  session,
  signedIn,
}: {
  session: SessionResponse | null
  signedIn: boolean
}) {
  const [busy, setBusy] = useState(false)
  const identity = session?.identity
  const title = signedIn
    ? `Signed in as ${identity?.email ?? identity?.pairwise_sub ?? 'Shoo user'}`
    : 'Sign in with Shoo'

  async function toggleAuth() {
    setBusy(true)

    if (signedIn) {
      createScopeShooAuth().clearIdentity()
      await fetch('/auth/session', { method: 'DELETE' }).catch(() => undefined)
      window.location.assign('/')
      return
    }

    try {
      await createScopeShooAuth().startSignIn({ requestPii: true })
    } catch {
      setBusy(false)
    }
  }

  return (
    <Button
      aria-label={title}
      disabled={busy}
      onClick={() => void toggleAuth()}
      size="sm"
      title={title}
      variant={signedIn ? 'secondary' : 'default'}
    >
      {busy ? (
        <LoaderCircle className="size-3.5 animate-spin" />
      ) : signedIn ? (
        <LogOut className="size-3.5" />
      ) : (
        <LogIn className="size-3.5" />
      )}
      {!busy && <span className="hidden sm:inline">{signedIn ? 'Sign out' : 'Sign in'}</span>}
    </Button>
  )
}

function RepoActions({
  role,
}: {
  role: RepoRole | null
}) {
  const owner = role === 'Owner'

  return (
    <div className="flex w-full flex-wrap gap-2 sm:w-auto sm:justify-end">
      <Button
        asChild={owner}
        className="min-w-0 flex-1 sm:flex-none"
        disabled={!owner}
        size="sm"
        title={owner ? 'Repository settings' : 'Owner role required'}
        variant="secondary"
      >
        {owner ? (
          <Link to="/settings">
            <Settings className="size-3.5" />
            <span>Settings</span>
          </Link>
        ) : (
          <>
            <Settings className="size-3.5" />
            <span>Settings</span>
          </>
        )}
      </Button>
    </div>
  )
}

function WorkspaceError({ error }: ErrorComponentProps) {
  const message = error instanceof Error ? error.message : 'Unexpected route error'

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[720px]">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Scope failed to load</AlertTitle>
          <AlertDescription>{message}</AlertDescription>
        </Alert>
      </div>
    </main>
  )
}

function WorkspaceAlerts({
  files,
  gitBoundary,
  gitProjection,
  projection,
  session,
}: {
  files: LoadState<RepoFile[]>
  gitBoundary: GitBoundaryState
  gitProjection: LoadState<GitProjection>
  projection: LoadState<Projection>
  session: LoadState<SessionResponse>
}) {
  const errors = [
    session.error && `Session: ${session.error}`,
    files.error && `Files: ${files.error}`,
    projection.error && `Projection: ${projection.error}`,
    gitProjection.error && `Objects: ${gitProjection.error}`,
    gitBoundary.state !== 'explicit' && `Git: ${gitBoundary.detail}`,
  ].filter(Boolean)

  if (errors.length === 0) {
    return null
  }

  return (
    <Alert className="mt-6" variant="destructive">
      <AlertCircle className="size-4" />
      <AlertTitle>Repository state is incomplete</AlertTitle>
      <AlertDescription>{errors.join(' ')}</AlertDescription>
    </Alert>
  )
}

function MetricStrip({
  blobs,
  commits,
  paths,
  writeEnabled,
}: {
  blobs: number
  commits: number
  paths: number
  writeEnabled: boolean
}) {
  return (
    <dl className="grid grid-cols-2 gap-px overflow-hidden border-y border-border bg-border sm:grid-cols-4">
      <Metric label="Paths" value={paths} />
      <Metric label="Commits" value={commits} />
      <Metric label="Objects" value={blobs} />
      <Metric label="Write" value={writeEnabled ? 'Allowed' : 'Blocked'} />
    </dl>
  )
}

function Metric({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="bg-background px-3 py-4">
      <dt className="text-xs leading-4 text-muted-foreground">{label}</dt>
      <dd className="mt-1 truncate text-lg font-semibold leading-7">{value}</dd>
    </div>
  )
}

function SectionTitle({
  action,
  title,
}: {
  action?: ReactNode
  title: string
}) {
  return (
    <div className="mb-3 flex min-h-8 items-center justify-between gap-3">
      <h2 className="text-sm font-semibold leading-5">{title}</h2>
      {action}
    </div>
  )
}

function RepoFileTable({
  canManageFiles,
  files,
  onSetVisibility,
  pendingFile,
}: {
  canManageFiles: boolean
  files: LoadState<RepoFile[]>
  onSetVisibility: (file: RepoFile, visibility: FileVisibility) => void
  pendingFile: string | null
}) {
  const repoFiles = files.data ?? []

  if (files.error) {
    return (
      <Alert variant="destructive">
        <AlertCircle className="size-4" />
        <AlertTitle>Git files unavailable</AlertTitle>
        <AlertDescription>{files.error}</AlertDescription>
      </Alert>
    )
  }

  if (repoFiles.length === 0) {
    return <EmptyState label="No files visible" />
  }

  return (
    <div className="overflow-hidden rounded-md border border-border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Path</TableHead>
            <TableHead className="w-32 sm:w-40">Object</TableHead>
            <TableHead className="w-40 text-right">Visibility</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {repoFiles.map((file) => (
            <TableRow key={`${file.path}-${file.oid}`}>
              <TableCell className="max-w-[220px] truncate font-mono text-xs sm:max-w-[520px]">
                {file.path}
              </TableCell>
              <TableCell className="font-mono text-xs text-muted-foreground">
                {shortOid(file.oid)}
              </TableCell>
              <TableCell className="text-right">
                <VisibilityToggle
                  disabledReason={visibilityDisabledReason(
                    file,
                    canManageFiles,
                    pendingFile,
                  )}
                  file={file}
                  onSetVisibility={onSetVisibility}
                />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

function VisibilityConfirmDialog({
  confirmation,
  onCancel,
  onConfirm,
}: {
  confirmation: PendingVisibilityChange | null
  onCancel: () => void
  onConfirm: () => void
}) {
  if (!confirmation) {
    return null
  }

  const { file, visibility } = confirmation

  return (
    <div
      aria-labelledby="visibility-confirm-title"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 px-4 backdrop-blur-sm"
      role="alertdialog"
    >
      <div className="w-full max-w-md rounded-md border border-border bg-background p-5 shadow-lg">
        <h2
          className="text-base font-semibold leading-6"
          id="visibility-confirm-title"
        >
          Make file {visibility.toLowerCase()}?
        </h2>
        <p className="mt-2 break-words font-mono text-sm leading-6 text-muted-foreground">
          {file.path}
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <Button
            onClick={onCancel}
            type="button"
            variant="secondary"
          >
            Cancel
          </Button>
          <Button onClick={onConfirm} type="button">Confirm</Button>
        </div>
      </div>
    </div>
  )
}

function VisibilityToggle({
  disabledReason,
  file,
  onSetVisibility,
}: {
  disabledReason: string | null
  file: RepoFile
  onSetVisibility: (file: RepoFile, visibility: FileVisibility) => void
}) {
  const isPublic = file.visibility === 'Public'
  const nextVisibility = isPublic ? 'Private' : 'Public'
  const disabled = Boolean(disabledReason)

  return (
    <button
      aria-checked={isPublic}
      aria-label={`Make ${file.path} ${nextVisibility.toLowerCase()}`}
      className={cn(
        'inline-flex h-7 w-[92px] items-center justify-between rounded-full border px-1 text-[11px] font-medium transition-colors',
        isPublic
          ? 'border-green-400 bg-green-100 text-green-900'
          : 'border-border bg-muted text-muted-foreground',
        disabled && 'cursor-not-allowed opacity-55',
      )}
      disabled={disabled}
      onClick={() => onSetVisibility(file, nextVisibility)}
      role="switch"
      title={disabledReason ?? `Make ${file.path} ${nextVisibility.toLowerCase()}`}
      type="button"
    >
      <span className={cn('px-2', !isPublic && 'order-2')}>{file.visibility}</span>
      <span className="size-5 rounded-full bg-background shadow-sm" />
    </button>
  )
}

function visibilityDisabledReason(
  file: RepoFile,
  canManageFiles: boolean,
  pendingFile: string | null,
) {
  if (!file.tracked) {
    return 'Track this file in Git before changing visibility'
  }
  if (!canManageFiles) {
    return 'Owner role required'
  }
  if (pendingFile === file.path) {
    return 'Updating visibility'
  }

  return null
}

function SessionPanel({
  canWrite,
  principal,
  session,
  signedIn,
}: {
  canWrite: boolean
  principal: PrincipalId
  session: SessionResponse | null
  signedIn: boolean
}) {
  const identity = session?.identity
  const access = session?.repo.role ?? 'Public'
  const subject = identity?.email ?? identity?.pairwise_sub ?? 'Anonymous'
  const verified = identity?.email_verified ?? false

  return (
    <section className="border-t border-border pt-4">
      <SectionTitle title="Session" />
      <dl className="space-y-3 text-sm">
        <KeyValue label="Subject" value={signedIn ? subject : 'Anonymous'} />
        <KeyValue label="Principal" value={principal} />
        <KeyValue label="Access" value={access} />
        <KeyValue label="Email" value={verified ? 'Verified' : 'Not verified'} />
        <KeyValue label="Read" value={session?.capabilities.read ? 'Allowed' : 'Blocked'} />
        <KeyValue label="Write" value={canWrite ? 'Allowed' : 'Blocked'} />
      </dl>
    </section>
  )
}

function CommitList({ commits }: { commits: ProjectedCommit[] }) {
  if (commits.length === 0) {
    return <EmptyState label="No commits visible" />
  }

  return (
    <div className="divide-y divide-border border-y border-border">
      {commits.map((commit) => (
        <div
          className="grid gap-2 py-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
          key={commit.projected_id}
        >
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold leading-5">
              {commit.message}
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap gap-x-3 gap-y-1 text-xs leading-4 text-muted-foreground">
              <span className="truncate">{commit.author ?? 'author hidden'}</span>
              <span>{commit.changes.length} changes</span>
              <span className="font-mono">{shortOid(commit.projected_id)}</span>
            </div>
          </div>
          <Badge variant={commit.synthetic ? 'secondary' : 'outline'}>
            {commit.synthetic ? 'Synthetic' : 'Canonical'}
          </Badge>
        </div>
      ))}
    </div>
  )
}

function KeyValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[84px_minmax(0,1fr)] gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate font-medium">{value}</dd>
    </div>
  )
}

function EmptyState({ label }: { label: string }) {
  return (
    <div className="rounded-md border border-dashed border-border px-3 py-8 text-center text-sm text-muted-foreground">
      {label}
    </div>
  )
}

function StatusBadge({
  children,
  tone,
}: {
  children: ReactNode
  tone: 'good' | 'bad' | 'neutral'
}) {
  return (
    <Badge
      className={cn(
        tone === 'good' && 'border-green-400 bg-green-100 text-green-900',
        tone === 'bad' && 'border-red-400 bg-red-100 text-red-900',
      )}
      variant={tone === 'neutral' ? 'outline' : 'default'}
    >
      {children}
    </Badge>
  )
}

async function loadWorkspace(
  idToken?: string,
  signal?: AbortSignal,
): Promise<WorkspaceData> {
  const api = getApiConnection()
  const init = {
    headers: authHeaders(idToken),
    signal,
  }
  const [session, files, projection, gitProjection, gitBoundary] = await Promise.all([
    safeLoadJson<SessionResponse>(
      `${api}/v1/repos/${repoOwner}/${repoName}/session`,
      init,
    ),
    safeLoadJson<RepoFile[]>(
      `${api}/v1/repos/${repoOwner}/${repoName}/files`,
      init,
    ),
    safeLoadJson<Projection>(
      `${api}/v1/repos/${repoOwner}/${repoName}/projections`,
      init,
    ),
    safeLoadJson<GitProjection>(
      `${api}/v1/repos/${repoOwner}/${repoName}/git-projections`,
      init,
    ),
    loadGitBoundary(api, idToken, signal),
  ])

  return {
    files,
    gitBoundary,
    gitProjection: stripGitProjectionContent(gitProjection),
    projection: stripProjectionContent(projection),
    session,
  }
}

function stripProjectionContent(
  projection: LoadState<Projection>,
): LoadState<Projection> {
  if (!projection.data) {
    return projection
  }

  return {
    ...projection,
    data: {
      ...projection.data,
      commits: projection.data.commits.map((commit) => ({
        ...commit,
        changes: commit.changes.map(({ path }) => ({ path })),
      })),
    },
  }
}

function stripGitProjectionContent(
  gitProjection: LoadState<GitProjection>,
): LoadState<GitProjection> {
  if (!gitProjection.data) {
    return gitProjection
  }

  return {
    ...gitProjection,
    data: {
      ...gitProjection.data,
      blobs: gitProjection.data.blobs.map(({ oid, path }) => ({ oid, path })),
    },
  }
}

async function safeLoadJson<T>(
  url: string,
  init?: RequestInit,
): Promise<LoadState<T>> {
  try {
    return {
      data: await loadJson<T>(url, init),
      error: null,
      loading: false,
      status: 200,
    }
  } catch (error) {
    return {
      data: null,
      error: error instanceof Error ? error.message : 'request failed',
      loading: false,
      status: error instanceof HttpError ? error.status : null,
    }
  }
}

async function loadGitBoundary(
  baseUrl: string,
  idToken?: string,
  signal?: AbortSignal,
): Promise<GitBoundaryState> {
  try {
    const response = await fetch(
      `${baseUrl}/git/${repoOwner}/${repoName}/info/refs?service=git-upload-pack`,
      {
        headers: authHeaders(idToken),
        signal,
      },
    )
    const body = await response.json().catch(() => null)

    if (response.status === 501) {
      return {
        state: 'explicit',
        detail:
          body?.next ?? 'Git clone is blocked until real packfile serving exists.',
      }
    }

    return {
      state: 'unexpected',
      detail: `unexpected status ${response.status}`,
    }
  } catch (error) {
    return {
      state: 'error',
      detail:
        error instanceof Error ? error.message : 'git boundary check failed',
    }
  }
}

async function readRequestAuthToken() {
  const { getCookie } = await import('@tanstack/react-start/server')
  return getCookie(authCookieName)
}

function parseSetFileVisibilityInput(input: unknown): SetFileVisibilityInput {
  const data = input as Partial<SetFileVisibilityInput> | null
  const path = typeof data?.path === 'string' ? data.path.trim() : ''
  const visibility = data?.visibility

  if (!path.startsWith('/')) {
    throw new Error('File path must be absolute.')
  }

  if (visibility !== 'Public' && visibility !== 'Private') {
    throw new Error('Visibility must be Public or Private.')
  }

  return { path, visibility }
}

async function loadJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init)
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new HttpError(
      payload?.error ?? `request failed: ${response.status}`,
      response.status,
    )
  }

  return payload as T
}

class HttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message)
  }
}

function authHeaders(idToken?: string): HeadersInit {
  return idToken ? { authorization: `Bearer ${idToken}` } : {}
}

function shortOid(value: string) {
  return value.slice(0, 12)
}

function applyTheme(theme: ThemeMode) {
  if (typeof document === 'undefined') {
    return
  }

  document.documentElement.classList.toggle('dark', theme === 'dark')
  document.documentElement.style.colorScheme = theme
}

function getApiConnection() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error('Set VITE_SCOPE_API_URL before loading repository state.')
}

function getApiMutationConnection() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error('Set VITE_SCOPE_API_URL before changing repository state.')
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
