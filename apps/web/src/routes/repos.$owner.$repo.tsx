import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
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
import { authCookieName } from '@/lib/auth'
import { cn } from '@/lib/utils'
import { Link, Outlet, createFileRoute, useChildMatches } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  AlertCircle,
  ArrowLeft,
  ArrowRight,
  FileSearch,
  GitBranch,
  Globe2,
  Lock,
} from 'lucide-react'

type Visibility = 'Private' | 'Public'
type RepoRole = 'Reader' | 'Writer' | 'Maintainer' | 'Owner'
type RepoLifecycleState = 'PendingFirstPush' | 'PendingPublish' | 'Published'

type RepoSummary = {
  id: string
  owner_handle: string
  name: string
  lifecycle_state: RepoLifecycleState
  default_visibility: Visibility
  role: RepoRole
  staged_update_pending: boolean
}

type RepoFile = {
  path: string
  oid: string
  tracked: boolean
  visibility: Visibility
}

type RepoDetail = {
  files: RepoFile[]
  kind: 'repo'
  repo: RepoSummary
}

type RepoDetailState =
  | RepoDetail
  | {
      kind: 'signedOut'
    }

type RepoParams = {
  owner: string
  repo: string
}

const localApiBase = 'http://localhost:8080'

const loadRepoForRequest = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      return { kind: 'signedOut' } satisfies RepoDetailState
    }

    const api = getApiConnection()
    const init = { headers: authHeaders(idToken) }
    const [repo, files] = await Promise.all([
      loadJson<RepoSummary>(`${api}/v1/repos/${data.owner}/${data.repo}`, init),
      loadJson<RepoFile[]>(`${api}/v1/repos/${data.owner}/${data.repo}/files`, init),
    ])

    return { files, kind: 'repo', repo } satisfies RepoDetailState
  })

export const Route = createFileRoute('/repos/$owner/$repo')({
  loader: ({ params }) => loadRepoForRequest({ data: params }),
  errorComponent: RepoDetailError,
  component: RepoDetailPage,
})

function RepoDetailPage() {
  const detail = Route.useLoaderData()
  const childMatches = useChildMatches()

  if (childMatches.length > 0) {
    return <Outlet />
  }

  if (detail.kind === 'signedOut') {
    return (
      <RepoDetailMessage
        message="Sign in to view this repository."
        title="Repository unavailable"
      />
    )
  }

  const { files, repo } = detail

  return (
    <main className="min-h-screen bg-background text-foreground">
      <header className="border-b border-border bg-background">
        <div className="mx-auto flex min-h-16 max-w-[980px] items-center justify-between gap-3 px-4 py-3 sm:px-6">
          <Link className="flex min-w-0 items-center gap-3" to="/">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-md border border-border">
              <GitBranch className="size-4" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold leading-5">Scope</div>
              <div className="truncate font-mono text-xs leading-4 text-muted-foreground">
                {repo.id}
              </div>
            </div>
          </Link>
          <Button asChild size="sm" variant="secondary">
            <Link to="/">
              <ArrowLeft className="size-3.5" />
              <span>Repos</span>
            </Link>
          </Button>
        </div>
      </header>

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Badge variant="outline">{lifecycleLabel(repo.lifecycle_state)}</Badge>
              <VisibilityBadge visibility={repo.default_visibility} />
              {repo.staged_update_pending && (
                <Badge variant="outline">Staged update</Badge>
              )}
            </div>
            <h1 className="truncate font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              {repo.id}
            </h1>
          </div>
          <RepoAction repo={repo} />
        </div>

        <section className="mt-8 border-y border-border">
          {files.length === 0 ? (
            <div className="flex items-center gap-3 py-10">
              <div className="flex size-10 shrink-0 items-center justify-center rounded-md border border-border">
                <FileSearch className="size-5 text-muted-foreground" />
              </div>
              <div className="min-w-0">
                <div className="text-sm font-medium leading-5">No live files</div>
                <div className="mt-1 text-sm leading-5 text-muted-foreground">
                  {repo.lifecycle_state === 'PendingPublish'
                    ? 'Review the pending import before publishing.'
                    : 'Files will appear here after the repo has published content.'}
                </div>
              </div>
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>File</TableHead>
                  <TableHead className="w-[120px]">Visibility</TableHead>
                  <TableHead className="w-[90px]">Git</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {files.map((file) => (
                  <TableRow key={file.path}>
                    <TableCell className="max-w-[460px] truncate font-mono text-xs sm:max-w-[700px]">
                      {file.path}
                    </TableCell>
                    <TableCell>
                      <VisibilityBadge visibility={file.visibility} />
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">
                        {file.tracked ? 'Tracked' : 'Untracked'}
                      </Badge>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </section>
      </section>
    </main>
  )
}

function RepoAction({ repo }: { repo: RepoSummary }) {
  if (repo.lifecycle_state === 'PendingFirstPush') {
    return (
      <Button asChild size="sm">
        <Link
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/repos/$owner/$repo/setup"
        >
          <ArrowRight className="size-3.5" />
          <span>Setup</span>
        </Link>
      </Button>
    )
  }

  if (repo.lifecycle_state === 'PendingPublish' || repo.staged_update_pending) {
    return (
      <Button asChild size="sm">
        <Link
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/repos/$owner/$repo/review"
        >
          <ArrowRight className="size-3.5" />
          <span>Review</span>
        </Link>
      </Button>
    )
  }

  return null
}

function VisibilityBadge({ visibility }: { visibility: Visibility }) {
  return (
    <Badge
      className={cn(
        visibility === 'Private' && 'border-amber-400 bg-amber-100 text-amber-900',
        visibility === 'Public' && 'border-green-400 bg-green-100 text-green-900',
      )}
      variant="outline"
    >
      {visibility === 'Private' ? (
        <Lock className="size-3" />
      ) : (
        <Globe2 className="size-3" />
      )}
      {visibility}
    </Badge>
  )
}

function RepoDetailError({ error }: { error: unknown }) {
  const message =
    error instanceof Error ? error.message : 'Unexpected repository error'

  return (
    <RepoDetailMessage message={message} title="Repository unavailable" />
  )
}

function RepoDetailMessage({
  message,
  title,
}: {
  message: string
  title: string
}) {
  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[760px] border-y border-border py-6">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>{title}</AlertTitle>
          <AlertDescription>{message}</AlertDescription>
        </Alert>
        <Button asChild className="mt-5" size="sm" variant="secondary">
          <Link to="/">
            <ArrowLeft className="size-3.5" />
            <span>Repos</span>
          </Link>
        </Button>
      </div>
    </main>
  )
}

async function readRequestAuthToken() {
  const { getCookie } = await import('@tanstack/react-start/server')
  return getCookie(authCookieName)
}

function parseRepoParams(input: unknown): RepoParams {
  const data = input as Partial<RepoParams> | null
  const owner = typeof data?.owner === 'string' ? data.owner.trim() : ''
  const repo = typeof data?.repo === 'string' ? data.repo.trim() : ''

  if (!owner || !repo) {
    throw new Error('Repository route is incomplete.')
  }

  return { owner, repo }
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

function lifecycleLabel(state: RepoLifecycleState) {
  switch (state) {
    case 'PendingFirstPush':
      return 'Pending first push'
    case 'PendingPublish':
      return 'Pending publish'
    case 'Published':
      return 'Published'
  }
}

function getApiConnection() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error('Set VITE_SCOPE_API_URL before loading repositories.')
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
