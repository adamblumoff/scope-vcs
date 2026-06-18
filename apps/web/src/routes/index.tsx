import {
  Alert,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Progress } from '@/components/ui/progress'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { cn } from '@/lib/utils'
import { createFileRoute } from '@tanstack/react-router'
import type { ErrorComponentProps } from '@tanstack/react-router'
import {
  AlertCircle,
  CheckCircle2,
  KeyRound,
  Layers3,
  Lock,
  Moon,
  RefreshCw,
  Server,
  ShieldCheck,
  Sun,
  Upload,
} from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'

type PrincipalId = 'public' | 'team-core' | 'owner'

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

type HealthResponse = {
  status: string
  service: string
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

type DashboardData = {
  baseUrl: string
  gitBoundary: GitBoundaryState
  gitProjection: LoadState<GitProjection>
  health: LoadState<HealthResponse>
  projection: LoadState<Projection>
}

type ThemeMode = 'dark' | 'light'

const repoId = 'scope-demo'
const deployedApiBase = 'https://scope-api-production-0251.up.railway.app'
const themeStorageKey = 'scope-theme'

const principals = [
  {
    id: 'public',
    label: 'Public',
    description: 'A shareable clone surface with private details removed.',
  },
  {
    id: 'team-core',
    label: 'Team Core',
    description: 'An internal view for authorized contributors.',
  },
  {
    id: 'owner',
    label: 'Owner',
    description: 'The full audit view for policy and projection checks.',
  },
] satisfies Array<{
  id: PrincipalId
  label: string
  description: string
}>

const principalById = Object.fromEntries(
  principals.map((principal) => [principal.id, principal]),
) as Record<PrincipalId, (typeof principals)[number]>

const principalCopy: Record<
  PrincipalId,
  {
    headline: string
    body: string
    proof: string
    manifestHint: string
  }
> = {
  public: {
    headline: 'Public Clone',
    body: 'Useful repository content without private names, bytes, authors, counts, or cadence.',
    proof: 'Private work is collapsed into a synthetic public commit.',
    manifestHint: 'Public writes should be rejected by policy.',
  },
  'team-core': {
    headline: 'Team Workspace',
    body: 'Internal implementation files appear only for principals that are allowed to receive them.',
    proof: 'The projected object set expands after the principal changes.',
    manifestHint: 'Authorized writes should produce a signed manifest.',
  },
  owner: {
    headline: 'Owner Audit',
    body: 'The canonical view for checking policy, object output, and write authorization.',
    proof: 'All demo policy outcomes are inspectable from this view.',
    manifestHint: 'Owner writes should produce a signed manifest.',
  },
}

export const Route = createFileRoute('/')({
  validateSearch: (search: Record<string, unknown>) => ({
    principal: parsePrincipal(search.principal),
  }),
  loaderDeps: ({ search }) => ({ principal: search.principal }),
  loader: ({ deps }) => loadDashboard(deps.principal),
  pendingComponent: DashboardPending,
  errorComponent: DashboardError,
  component: ScopeDashboard,
})

function ScopeDashboard() {
  const { principal } = Route.useSearch()
  const navigate = Route.useNavigate()
  const dashboard = Route.useLoaderData()
  const [manifest, setManifest] = useState<LoadState<ManifestResponse>>({
    data: null,
    error: null,
    loading: false,
  })
  const [theme, setTheme] = useState<ThemeMode>('dark')

  useEffect(() => {
    const nextTheme = readStoredTheme()
    setTheme(nextTheme)
    applyTheme(nextTheme)
  }, [])

  useEffect(() => {
    setManifest({ data: null, error: null, loading: false })
  }, [principal])

  const copy = principalCopy[principal]
  const selectedPrincipal = principalById[principal]
  const visiblePaths = useMemo(
    () =>
      dashboard.projection.data
        ? visibleProjectionPaths(dashboard.projection.data)
        : [],
    [dashboard.projection.data],
  )
  const apiOnline = Boolean(dashboard.health.data && !dashboard.health.error)

  function selectPrincipal(nextPrincipal: string) {
    void navigate({
      search: { principal: parsePrincipal(nextPrincipal) },
    })
  }

  function toggleTheme() {
    const nextTheme = theme === 'dark' ? 'light' : 'dark'
    setTheme(nextTheme)
    applyTheme(nextTheme)
    window.localStorage.setItem(themeStorageKey, nextTheme)
  }

  async function createManifest() {
    setManifest({ data: null, error: null, loading: true })
    const changed_paths =
      principal === 'team-core' ? ['/internal/model.rs'] : ['/README.md']

    try {
      const response = await fetch(`${dashboard.baseUrl}/v1/repos/${repoId}/push-manifests`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          principal_id: principal,
          device_id: 'web-demo',
          commit_graph_hash: `${principal}-demo-graph`,
          changed_paths,
          mixed_policy: 'SyntheticPublicCommit',
        }),
      })
      const payload = await response.json().catch(() => null)

      if (!response.ok) {
        throw new Error(payload?.error ?? `request failed: ${response.status}`)
      }

      setManifest({
        data: payload as ManifestResponse,
        error: null,
        loading: false,
      })
    } catch (error) {
      setManifest({
        data: null,
        error: error instanceof Error ? error.message : 'manifest failed',
        loading: false,
      })
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <header className="border-b border-border bg-card">
        <div className="mx-auto flex h-16 max-w-[1200px] items-center justify-between px-4 sm:px-6">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex size-8 items-center justify-center rounded-md border border-border bg-card shadow-[0_2px_2px_rgba(0,0,0,0.04)]">
              <Layers3 className="size-4" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold leading-5">Scope</div>
              <div className="truncate text-xs leading-4 text-muted-foreground">
                {repoId}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <div className="hidden items-center gap-2 min-[520px]:flex">
              <ServiceBadge label="Web" ready />
              <ServiceBadge label="API" ready={apiOnline} />
              <ServiceBadge
                label="Git"
                ready={dashboard.gitBoundary.state === 'explicit'}
              />
            </div>
            <ThemeToggle theme={theme} toggleTheme={toggleTheme} />
          </div>
        </div>
      </header>

      <section className="mx-auto max-w-[1200px] px-4 py-8 sm:px-6 lg:py-10">
        <div className="mb-8 flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
          <div className="max-w-2xl">
            <div className="mb-2 flex items-center gap-2 text-xs leading-4 text-muted-foreground">
              <Server className="size-3.5" />
              <span>{dashboard.baseUrl.replace(/^https?:\/\//, '')}</span>
            </div>
            <h1 className="text-[32px] font-semibold leading-10 tracking-[-1.28px]">
              Projection Dashboard
            </h1>
            <p className="mt-2 max-w-xl text-sm leading-5 text-muted-foreground">
              See how one repository becomes different Git-safe views depending
              on who is asking.
            </p>
          </div>

          <Tabs
            className="w-full md:w-auto"
            onValueChange={selectPrincipal}
            value={principal}
          >
            <TabsList className="grid h-10 w-full grid-cols-3 md:w-[360px]">
              {principals.map((item) => (
                <TabsTrigger key={item.id} value={item.id}>
                  {item.label}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>
        </div>

        <div className="grid gap-4 lg:grid-cols-[1.45fr_0.85fr]">
          <Card className="shadow-[var(--shadow-card)]">
            <CardHeader className="border-b border-border pb-6">
              <div>
                <Badge className="mb-3" variant="outline">
                  {selectedPrincipal.label}
                </Badge>
                <CardTitle className="text-2xl font-semibold leading-8 tracking-[-0.96px]">
                  {copy.headline}
                </CardTitle>
                <CardDescription className="mt-2 max-w-2xl leading-5">
                  {copy.body}
                </CardDescription>
              </div>
              <CardAction>
                <StatusBadge
                  ready={!dashboard.projection.error}
                  text={dashboard.projection.error ? 'Unavailable' : 'Ready'}
                />
              </CardAction>
            </CardHeader>

            <CardContent className="space-y-6">
              {dashboard.projection.error ? (
                <Alert variant="destructive">
                  <AlertCircle className="size-4" />
                  <AlertTitle>Projection Unavailable</AlertTitle>
                  <AlertDescription>{dashboard.projection.error}</AlertDescription>
                </Alert>
              ) : (
                <Alert className="border-green-400 bg-green-100 text-green-1000">
                  <ShieldCheck className="size-4 text-green-900" />
                  <AlertTitle>{copy.proof}</AlertTitle>
                  <AlertDescription className="text-green-900">
                    The route loader read the live projection API for this
                    principal.
                  </AlertDescription>
                </Alert>
              )}

              <div className="grid grid-cols-3 divide-x divide-border overflow-hidden rounded-md border border-border">
                <Metric label="Visible Paths" value={visiblePaths.length} />
                <Metric
                  label="Visible Commits"
                  value={dashboard.projection.data?.commits.length ?? 0}
                />
                <Metric
                  label="Virtual Blobs"
                  value={dashboard.gitProjection.data?.blobs.length ?? 0}
                />
              </div>
            </CardContent>
          </Card>

          <div className="space-y-4">
            <WriteReadiness
              createManifest={createManifest}
              manifest={manifest}
              manifestHint={copy.manifestHint}
              principal={principal}
            />
            <Guardrails
              gitBoundary={dashboard.gitBoundary}
              principal={principal}
            />
          </div>
        </div>

        <div className="mt-4 grid gap-4 lg:grid-cols-[1.1fr_0.9fr]">
          <ObjectSet
            gitProjection={dashboard.gitProjection}
            visiblePaths={visiblePaths}
          />
          <CommitHistory projection={dashboard.projection.data} />
        </div>
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
      aria-label={`Switch to ${nextTheme} Mode`}
      onClick={toggleTheme}
      size="sm"
      title={`Switch to ${nextTheme} Mode`}
      variant="secondary"
    >
      {theme === 'dark' ? (
        <Sun className="size-3.5" />
      ) : (
        <Moon className="size-3.5" />
      )}
      <span className="hidden sm:inline">{nextTheme}</span>
    </Button>
  )
}

function DashboardPending() {
  return (
    <main className="min-h-screen bg-background text-foreground">
      <header className="border-b border-border bg-card">
        <div className="mx-auto flex h-16 max-w-[1200px] items-center justify-between px-4 sm:px-6">
          <div className="flex items-center gap-3">
            <Skeleton className="size-8 rounded-md" />
            <div className="space-y-1.5">
              <Skeleton className="h-4 w-16" />
              <Skeleton className="h-3 w-24" />
            </div>
          </div>
          <div className="flex gap-2">
            <Skeleton className="h-5 w-12 rounded-full" />
            <Skeleton className="h-5 w-12 rounded-full" />
            <Skeleton className="h-5 w-12 rounded-full" />
          </div>
        </div>
      </header>
      <section className="mx-auto max-w-[1200px] px-4 py-8 sm:px-6 lg:py-10">
        <Skeleton className="h-[720px] w-full rounded-md" />
      </section>
    </main>
  )
}

function DashboardError({ error }: ErrorComponentProps) {
  const message = error instanceof Error ? error.message : 'Unexpected route error'

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[720px]">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Dashboard Failed</AlertTitle>
          <AlertDescription>
            {message}. Reload the page or switch principals and try again.
          </AlertDescription>
        </Alert>
      </div>
    </main>
  )
}

function Guardrails({
  gitBoundary,
  principal,
}: {
  gitBoundary: GitBoundaryState
  principal: PrincipalId
}) {
  const gitReady = gitBoundary.state === 'explicit'

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Lock className="size-4 text-muted-foreground" />
          Guardrails
        </CardTitle>
        <CardDescription>
          Policy checks run before a projected object is emitted.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-3">
          <GuardrailRow
            ok
            text={
              principal === 'public'
                ? 'Public output omits private structure.'
                : 'Output is scoped to this principal.'
            }
          />
          <GuardrailRow ok text="ACLs are checked before object projection." />
          <GuardrailRow
            ok={gitReady}
            text={
              gitReady
                ? 'Git clone returns an explicit 501 until packfiles exist.'
                : gitBoundary.detail
            }
          />
        </div>
        <Progress
          className="[&_[data-slot=progress-indicator]]:bg-blue-700"
          value={gitReady ? 100 : 55}
        />
      </CardContent>
    </Card>
  )
}

function WriteReadiness({
  createManifest,
  manifest,
  manifestHint,
  principal,
}: {
  createManifest: () => void
  manifest: LoadState<ManifestResponse>
  manifestHint: string
  principal: PrincipalId
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <KeyRound className="size-4 text-muted-foreground" />
          Write Readiness
        </CardTitle>
        <CardDescription>{manifestHint}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <Button
          className="w-full"
          disabled={manifest.loading}
          onClick={createManifest}
          variant="secondary"
        >
          {manifest.loading ? (
            <RefreshCw className="size-4 animate-spin" />
          ) : (
            <Upload className="size-4" />
          )}
          {manifest.loading ? 'Creating...' : 'Create Demo Manifest'}
        </Button>
        <ManifestResult manifest={manifest} principal={principal} />
      </CardContent>
    </Card>
  )
}

function ObjectSet({
  gitProjection,
  visiblePaths,
}: {
  gitProjection: LoadState<GitProjection>
  visiblePaths: string[]
}) {
  const blobs = gitProjection.data?.blobs ?? []

  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>Virtual Git Object Set</CardTitle>
          <CardDescription>
            The blobs this principal can receive.
          </CardDescription>
        </div>
        <CardAction>
          <Badge variant="outline">{visiblePaths.length} paths</Badge>
        </CardAction>
      </CardHeader>
      <CardContent>
        {gitProjection.error ? (
          <Alert variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Objects Unavailable</AlertTitle>
            <AlertDescription>{gitProjection.error}</AlertDescription>
          </Alert>
        ) : blobs.length > 0 ? (
          <div className="overflow-hidden rounded-md border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Path</TableHead>
                  <TableHead className="w-36">Virtual OID</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {blobs.map((blob) => (
                  <TableRow key={`${blob.path}-${blob.oid}`}>
                    <TableCell className="max-w-[260px] truncate font-mono text-xs">
                      {blob.path}
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {blob.oid.slice(0, 12)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            No virtual blobs are visible to this principal.
          </p>
        )}
      </CardContent>
    </Card>
  )
}

function CommitHistory({ projection }: { projection: Projection | null }) {
  const commits = projection?.commits.slice().reverse() ?? []

  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>Visible History</CardTitle>
          <CardDescription>Recent commits in the selected view.</CardDescription>
        </div>
        <CardAction>
          <Badge variant="outline">{projection?.commits.length ?? 0} commits</Badge>
        </CardAction>
      </CardHeader>
      <CardContent>
        {commits.length > 0 ? (
          <div className="space-y-3">
            {commits.map((commit) => (
              <div
                className="rounded-md border border-border bg-card p-3"
                key={commit.projected_id}
              >
                <div className="mb-2 flex items-center justify-between gap-3">
                  <span className="min-w-0 truncate text-sm font-semibold leading-5">
                    {commit.message}
                  </span>
                  <Badge variant={commit.synthetic ? 'secondary' : 'outline'}>
                    {commit.synthetic ? 'Synthetic' : 'Canonical'}
                  </Badge>
                </div>
                <div className="flex items-center justify-between gap-3 text-xs leading-4 text-muted-foreground">
                  <span className="truncate">
                    {commit.author ?? 'author hidden'}
                  </span>
                  <span>{commit.changes.length} changes</span>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            No commits are visible to this principal.
          </p>
        )}
      </CardContent>
    </Card>
  )
}

function ManifestResult({
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
        <AlertTitle>Manifest Rejected</AlertTitle>
        <AlertDescription>
          {principal}: {manifest.error}
        </AlertDescription>
      </Alert>
    )
  }

  if (!manifest.data) {
    return (
      <p className="text-sm leading-5 text-muted-foreground">
        Run a demo request to confirm the selected principal&apos;s write policy.
      </p>
    )
  }

  const signed = manifest.data.signed_manifest

  return (
    <Alert className="border-green-400 bg-green-100 text-green-1000">
      <CheckCircle2 className="size-4 text-green-900" />
      <AlertTitle>Signed Manifest Created</AlertTitle>
      <AlertDescription className="space-y-1 text-green-900">
        <div className="truncate font-mono text-xs">{signed.manifest.id}</div>
        <div className="truncate font-mono text-xs">
          {signed.signature_hex.slice(0, 32)}...
        </div>
      </AlertDescription>
    </Alert>
  )
}

function Metric({ label, value }: { label: string; value: number }) {
  return (
    <div className="p-3">
      <div className="text-xs leading-4 text-muted-foreground">{label}</div>
      <div className="mt-1 text-xl font-semibold leading-8 tracking-[-0.4px]">
        {value}
      </div>
    </div>
  )
}

function ServiceBadge({ label, ready }: { label: string; ready: boolean }) {
  return (
    <Badge
      className={cn(
        'gap-1.5',
        ready
          ? 'border-green-400 bg-green-100 text-green-900'
          : 'border-red-400 bg-red-100 text-red-900',
      )}
      variant="outline"
    >
      {ready ? (
        <CheckCircle2 className="size-3" />
      ) : (
        <AlertCircle className="size-3" />
      )}
      {label}
    </Badge>
  )
}

function StatusBadge({ ready, text }: { ready: boolean; text: string }) {
  return (
    <Badge
      className={cn(
        ready
          ? 'border-green-400 bg-green-100 text-green-900'
          : 'border-red-400 bg-red-100 text-red-900',
      )}
      variant="outline"
    >
      {text}
    </Badge>
  )
}

function GuardrailRow({ ok, text }: { ok: boolean; text: string }) {
  return (
    <div className="flex items-start gap-3 text-sm leading-5">
      {ok ? (
        <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-green-900" />
      ) : (
        <AlertCircle className="mt-0.5 size-4 shrink-0 text-amber-900" />
      )}
      <span className="text-muted-foreground">{text}</span>
    </div>
  )
}

async function loadDashboard(principal: PrincipalId): Promise<DashboardData> {
  const baseUrl = getStaticApiBase()
  const [projection, gitProjection, health, gitBoundary] = await Promise.all([
    safeLoadJson<Projection>(
      `${baseUrl}/v1/repos/${repoId}/projections/${principal}`,
    ),
    safeLoadJson<GitProjection>(
      `${baseUrl}/v1/repos/${repoId}/git-projections/${principal}`,
    ),
    safeLoadJson<HealthResponse>(`${baseUrl}/healthz`),
    loadGitBoundary(baseUrl),
  ])

  return {
    baseUrl,
    gitBoundary,
    gitProjection,
    health,
    projection,
  }
}

async function safeLoadJson<T>(
  url: string,
): Promise<LoadState<T>> {
  try {
    return {
      data: await loadJson<T>(url),
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
): Promise<GitBoundaryState> {
  try {
    const response = await fetch(
      `${baseUrl}/git/acme/${repoId}/info/refs?service=git-upload-pack`,
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

async function loadJson<T>(url: string): Promise<T> {
  const response = await fetch(url)
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as T
}

function parsePrincipal(value: unknown): PrincipalId {
  return principals.some((principal) => principal.id === value)
    ? (value as PrincipalId)
    : 'public'
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

function getStaticApiBase() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  return deployedApiBase
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
