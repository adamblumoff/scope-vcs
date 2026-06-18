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
import { cn } from '@/lib/utils'
import { useShooAuth } from '@shoojs/react'
import { createFileRoute } from '@tanstack/react-router'
import type { ErrorComponentProps } from '@tanstack/react-router'
import {
  AlertCircle,
  CheckCircle2,
  GitBranch,
  Globe2,
  LogIn,
  LogOut,
  Moon,
  RefreshCw,
  ShieldCheck,
  Sun,
  Upload,
  UserPlus,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useMemo, useRef, useState } from 'react'

type PrincipalId = string
type RepoRole = 'Reader' | 'Writer' | 'Maintainer' | 'Owner'

type ProjectedChange = {
  path: string
  new_content: string | null
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
    content: string
  }>
  head_oid: string | null
}

type ManifestResponse = {
  signed_manifest: {
    manifest: {
      id: string
      repo_id: string
      principal_id: string
      device_id: string
      commit_graph_hash: string
      changed_paths: string[]
      mixed_policy: string
    }
    signature_hex: string
  }
}

type LoadState<T> = {
  data: T | null
  error: string | null
  loading: boolean
}

type GitBoundaryState = {
  state: 'explicit' | 'unexpected' | 'error'
  detail: string
}

type WorkspaceData = {
  api: ApiConnection
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
type ApiSource = 'env' | 'local-dev' | 'production-default'

type ApiConnection = {
  source: ApiSource
  url: string
}

const repoOwner = 'adamblumoff'
const repoName = 'scope-vcs'
const repoId = `${repoOwner}/${repoName}`
const localApiBase = 'http://localhost:8080'
const productionApiBase = 'https://scope-api-production-0251.up.railway.app'
const themeStorageKey = 'scope-theme'

export const Route = createFileRoute('/')({
  loader: () => loadWorkspace(),
  errorComponent: WorkspaceError,
  component: ScopeWorkspace,
})

function ScopeWorkspace() {
  const initialWorkspace = Route.useLoaderData()
  const [workspace, setWorkspace] = useState(initialWorkspace)
  const [manifest, setManifest] = useState<LoadState<ManifestResponse>>({
    data: null,
    error: null,
    loading: false,
  })
  const [theme, setTheme] = useState<ThemeMode>('dark')
  const auth = useShooAuth({
    shooBaseUrl: 'https://shoo.dev',
    callbackPath: '/',
    requestPii: true,
    autoSessionMonitor: true,
    sessionMonitorIntervalMs: 60_000,
  })
  const manifestAbortRef = useRef<AbortController | null>(null)
  const manifestRequestRef = useRef(0)
  const refreshAbortRef = useRef<AbortController | null>(null)
  const idToken = auth.identity.token

  useEffect(() => {
    const nextTheme = readStoredTheme()
    setTheme(nextTheme)
    applyTheme(nextTheme)
  }, [])

  useEffect(
    () => () => {
      manifestAbortRef.current?.abort()
      refreshAbortRef.current?.abort()
    },
    [],
  )

  useEffect(() => {
    manifestRequestRef.current += 1
    manifestAbortRef.current?.abort()
    manifestAbortRef.current = null
    setManifest({ data: null, error: null, loading: false })

    refreshAbortRef.current?.abort()
    refreshAbortRef.current = null

    if (!idToken) {
      setWorkspace(initialWorkspace)
      return
    }

    const controller = new AbortController()
    refreshAbortRef.current = controller

    loadWorkspace(idToken, controller.signal)
      .then((nextWorkspace) => {
        if (!controller.signal.aborted) {
          setWorkspace(nextWorkspace)
        }
      })
      .catch(() => {
        // Per-request failures are represented inside WorkspaceData. This
        // catches only unexpected top-level failures, so keep the last
        // coherent workspace on screen.
      })
      .finally(() => {
        if (!controller.signal.aborted) {
          refreshAbortRef.current = null
        }
      })

    return () => controller.abort()
  }, [idToken, initialWorkspace])

  const session = workspace.session.data
  const projection = workspace.projection.data
  const gitProjection = workspace.gitProjection.data
  const visiblePaths = useMemo(
    () => (projection ? visibleProjectionPaths(projection) : []),
    [projection],
  )
  const commits = useMemo(
    () => projection?.commits.slice().reverse() ?? [],
    [projection],
  )
  const role = session?.repo.role ?? null
  const roleLabel = role ?? 'Public'
  const principal = session?.principal_id ?? 'public'
  const canWrite = session?.capabilities.write ?? false
  const signedIn = Boolean(idToken)

  function toggleTheme() {
    const nextTheme = theme === 'dark' ? 'light' : 'dark'
    setTheme(nextTheme)
    applyTheme(nextTheme)
    window.localStorage.setItem(themeStorageKey, nextTheme)
  }

  async function createManifest() {
    const safetyError = getManifestSafetyError(workspace.api)
    if (safetyError) {
      setManifest({ data: null, error: safetyError, loading: false })
      return
    }

    if (!canWrite) {
      setManifest({
        data: null,
        error: 'This session cannot write to the repository.',
        loading: false,
      })
      return
    }

    const commitGraphHash = gitProjection?.head_oid
    if (!commitGraphHash) {
      setManifest({
        data: null,
        error: 'No projected Git head is available for this session.',
        loading: false,
      })
      return
    }

    manifestRequestRef.current += 1
    const requestId = manifestRequestRef.current
    manifestAbortRef.current?.abort()
    const controller = new AbortController()
    manifestAbortRef.current = controller
    setManifest({ data: null, error: null, loading: true })

    try {
      const response = await fetch(
        `${workspace.api.url}/v1/repos/${repoOwner}/${repoName}/push-manifests`,
        {
          method: 'POST',
          headers: {
            ...authHeaders(idToken),
            'content-type': 'application/json',
          },
          signal: controller.signal,
          body: JSON.stringify({
            device_id: 'web-console',
            commit_graph_hash: commitGraphHash,
            changed_paths: ['/README.md'],
            mixed_policy: 'SyntheticPublicCommit',
          }),
        },
      )
      const payload = await response.json().catch(() => null)

      if (!response.ok) {
        throw new Error(payload?.error ?? `request failed: ${response.status}`)
      }

      if (manifestRequestRef.current !== requestId || controller.signal.aborted) {
        return
      }

      setManifest({
        data: payload as ManifestResponse,
        error: null,
        loading: false,
      })
    } catch (error) {
      if (
        manifestRequestRef.current !== requestId ||
        controller.signal.aborted ||
        isAbortError(error)
      ) {
        return
      }

      setManifest({
        data: null,
        error: error instanceof Error ? error.message : 'manifest failed',
        loading: false,
      })
    } finally {
      if (manifestRequestRef.current === requestId) {
        manifestAbortRef.current = null
      }
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
              auth={auth}
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
            canWrite={canWrite}
            createManifest={createManifest}
            manifestLoading={manifest.loading}
            manifestReady={Boolean(manifest.data)}
            role={role}
          />
        </div>

        <WorkspaceAlerts
          gitBoundary={workspace.gitBoundary}
          gitProjection={workspace.gitProjection}
          projection={workspace.projection}
          session={workspace.session}
        />

        <MetricStrip
          blobs={gitProjection?.blobs.length ?? 0}
          commits={projection?.commits.length ?? 0}
          paths={visiblePaths.length}
          writeEnabled={canWrite}
        />

        <div className="grid gap-8 pt-8 lg:grid-cols-[minmax(0,1fr)_340px]">
          <section className="min-w-0">
            <SectionTitle
              action={<Badge variant="outline">{visiblePaths.length} paths</Badge>}
              title="Repository Files"
            />
            <ObjectTable gitProjection={workspace.gitProjection} />
          </section>

          <aside className="min-w-0 space-y-8">
            <SessionPanel
              canWrite={canWrite}
              principal={principal}
              session={session}
              signedIn={signedIn}
            />
            <ManifestPanel manifest={manifest} principal={principal} />
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
  auth,
  session,
  signedIn,
}: {
  auth: ReturnType<typeof useShooAuth>
  session: SessionResponse | null
  signedIn: boolean
}) {
  const identity = session?.identity
  const title = signedIn
    ? `Signed in as ${identity?.email ?? identity?.pairwise_sub ?? 'Shoo user'}`
    : 'Sign in with Shoo'

  async function toggleAuth() {
    if (signedIn) {
      auth.clearIdentity()
      return
    }

    await auth.signIn({ requestPii: true })
  }

  return (
    <Button
      aria-label={title}
      disabled={auth.loading}
      onClick={() => void toggleAuth()}
      size="sm"
      title={title}
      variant={signedIn ? 'secondary' : 'default'}
    >
      {signedIn ? <LogOut className="size-3.5" /> : <LogIn className="size-3.5" />}
      <span className="hidden sm:inline">
        {auth.loading ? 'Checking' : signedIn ? 'Sign out' : 'Sign in'}
      </span>
    </Button>
  )
}

function RepoActions({
  canWrite,
  createManifest,
  manifestLoading,
  manifestReady,
  role,
}: {
  canWrite: boolean
  createManifest: () => void
  manifestLoading: boolean
  manifestReady: boolean
  role: RepoRole | null
}) {
  const owner = role === 'Owner'

  return (
    <div className="flex w-full flex-wrap gap-2 sm:w-auto sm:justify-end">
      <Button
        className="min-w-0 flex-1 sm:flex-none"
        disabled={!canWrite || manifestLoading}
        onClick={createManifest}
        size="sm"
        title={canWrite ? 'Create push manifest' : 'Write access required'}
        variant={manifestReady ? 'secondary' : 'default'}
      >
        {manifestLoading ? (
          <RefreshCw className="size-3.5 animate-spin" />
        ) : manifestReady ? (
          <CheckCircle2 className="size-3.5" />
        ) : (
          <Upload className="size-3.5" />
        )}
        <span>Manifest</span>
      </Button>
      <Button
        className="min-w-0 flex-1 sm:flex-none"
        disabled
        size="sm"
        title={owner ? 'Invitation endpoint is not available yet' : 'Owner role required'}
        variant="secondary"
      >
        <UserPlus className="size-3.5" />
        <span>Invite</span>
      </Button>
      <Button
        className="min-w-0 flex-1 sm:flex-none"
        disabled
        size="sm"
        title={owner ? 'Publish endpoint is not available yet' : 'Owner role required'}
        variant="secondary"
      >
        <Globe2 className="size-3.5" />
        <span>Publish</span>
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
  gitBoundary,
  gitProjection,
  projection,
  session,
}: {
  gitBoundary: GitBoundaryState
  gitProjection: LoadState<GitProjection>
  projection: LoadState<Projection>
  session: LoadState<SessionResponse>
}) {
  const errors = [
    session.error && `Session: ${session.error}`,
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

function ObjectTable({
  gitProjection,
}: {
  gitProjection: LoadState<GitProjection>
}) {
  const blobs = gitProjection.data?.blobs ?? []

  if (gitProjection.error) {
    return (
      <Alert variant="destructive">
        <AlertCircle className="size-4" />
        <AlertTitle>Objects unavailable</AlertTitle>
        <AlertDescription>{gitProjection.error}</AlertDescription>
      </Alert>
    )
  }

  if (blobs.length === 0) {
    return <EmptyState label="No files visible" />
  }

  return (
    <div className="overflow-hidden rounded-md border border-border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Path</TableHead>
            <TableHead className="w-32 sm:w-40">Object</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {blobs.map((blob) => (
            <TableRow key={`${blob.path}-${blob.oid}`}>
              <TableCell className="max-w-[220px] truncate font-mono text-xs sm:max-w-[520px]">
                {blob.path}
              </TableCell>
              <TableCell className="font-mono text-xs text-muted-foreground">
                {shortOid(blob.oid)}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
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

function ManifestPanel({
  manifest,
  principal,
}: {
  manifest: LoadState<ManifestResponse>
  principal: PrincipalId
}) {
  if (manifest.error) {
    return (
      <Alert variant="destructive">
        <AlertCircle className="size-4" />
        <AlertTitle>Manifest rejected</AlertTitle>
        <AlertDescription>
          {principal}: {manifest.error}
        </AlertDescription>
      </Alert>
    )
  }

  if (!manifest.data) {
    return (
      <section className="border-t border-border pt-4">
        <SectionTitle title="Push Manifest" />
        <EmptyState label="No manifest created" />
      </section>
    )
  }

  const signed = manifest.data.signed_manifest

  return (
    <Alert className="border-green-400 bg-green-100 text-green-1000" live="polite">
      <CheckCircle2 className="size-4 text-green-900" />
      <AlertTitle>Manifest ready</AlertTitle>
      <AlertDescription className="space-y-1 text-green-900">
        <div className="truncate font-mono text-xs">{signed.manifest.id}</div>
        <div className="truncate font-mono text-xs">
          {signed.signature_hex.slice(0, 32)}...
        </div>
      </AlertDescription>
    </Alert>
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
  const [session, projection, gitProjection, gitBoundary] = await Promise.all([
    safeLoadJson<SessionResponse>(
      `${api.url}/v1/repos/${repoOwner}/${repoName}/session`,
      init,
    ),
    safeLoadJson<Projection>(
      `${api.url}/v1/repos/${repoOwner}/${repoName}/projections`,
      init,
    ),
    safeLoadJson<GitProjection>(
      `${api.url}/v1/repos/${repoOwner}/${repoName}/git-projections`,
      init,
    ),
    loadGitBoundary(api.url, idToken, signal),
  ])

  return {
    api,
    gitBoundary,
    gitProjection,
    projection,
    session,
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
    }
  } catch (error) {
    return {
      data: null,
      error: error instanceof Error ? error.message : 'request failed',
      loading: false,
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

function visibleProjectionPaths(projection: Projection) {
  const paths = projection.commits.flatMap((commit) =>
    commit.changes.map((change) => change.path),
  )
  return [...new Set(paths)].sort((left, right) => left.localeCompare(right))
}

async function loadJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init)
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as T
}

function authHeaders(idToken?: string): HeadersInit {
  return idToken ? { authorization: `Bearer ${idToken}` } : {}
}

function shortOid(value: string) {
  return value.slice(0, 12)
}

function readStoredTheme(): ThemeMode {
  if (typeof window === 'undefined') {
    return 'dark'
  }

  return window.localStorage.getItem(themeStorageKey) === 'light'
    ? 'light'
    : 'dark'
}

function applyTheme(theme: ThemeMode) {
  if (typeof document === 'undefined') {
    return
  }

  document.documentElement.classList.toggle('dark', theme === 'dark')
  document.documentElement.style.colorScheme = theme
}

function getApiConnection(): ApiConnection {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return { source: 'env', url: stripTrailingSlash(envBase) }
  }

  if (import.meta.env.DEV) {
    return { source: 'local-dev', url: localApiBase }
  }

  return { source: 'production-default', url: productionApiBase }
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}

function getManifestSafetyError(api: ApiConnection) {
  if (api.source !== 'production-default' || !isLocalBrowserHost()) {
    return null
  }

  return 'Set VITE_SCOPE_API_URL before creating manifests from a local production preview.'
}

function isLocalBrowserHost() {
  if (typeof window === 'undefined') {
    return false
  }

  return isLoopbackHost(window.location.hostname)
}

function isLoopbackHost(hostname: string) {
  const normalized = hostname.replace(/^\[|\]$/g, '')
  return ['localhost', '127.0.0.1', '::1'].includes(normalized)
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === 'AbortError'
}
