import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { authCookieName } from '@/lib/auth'
import { Link, createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  AlertCircle,
  ArrowLeft,
  GitBranch,
  KeyRound,
  LoaderCircle,
  RefreshCw,
  Terminal,
} from 'lucide-react'
import { useEffect, useState } from 'react'

type Visibility = 'Private' | 'Public'
type RepoRole = 'Reader' | 'Writer' | 'Maintainer' | 'Owner'
type RepoLifecycleState = 'PendingFirstPush' | 'PendingPublish' | 'Published'
type TokenStatus = 'Active' | 'Expired' | 'Used'

type RepoSummary = {
  id: string
  owner_handle: string
  name: string
  lifecycle_state: RepoLifecycleState
  default_visibility: Visibility
  role: RepoRole
}

type FirstPushToken = {
  status: TokenStatus
  created_at_unix: number
  expires_at_unix: number
  used_at_unix: number | null
  secret: string | null
}

type RepoSetup = {
  repo: RepoSummary
  git_remote_path: string
  remote_name: string
  push_branch: string
  push_enabled: boolean
  token: FirstPushToken | null
}

type RepoSetupView = RepoSetup & {
  commands: string[]
  git_remote_url: string
}

type SetupParams = {
  owner: string
  repo: string
}

const localApiBase = 'http://localhost:8080'

const loadSetupForRequest = createServerFn({ method: 'GET' })
  .validator(parseSetupParams)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      throw new Error('Sign in as the repo owner to view setup.')
    }

    const api = getApiConnection()
    const setup = await loadJson<RepoSetup>(
      `${api}/v1/repos/${data.owner}/${data.repo}/setup`,
      { headers: authHeaders(idToken) },
    )

    return setupView(api, setup)
  })

const regenerateTokenForRequest = createServerFn({ method: 'POST' })
  .validator(parseSetupParams)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      throw new Error('Sign in as the repo owner to regenerate setup token.')
    }

    const api = getApiMutationConnection()
    const response = await fetch(
      `${api}/v1/repos/${data.owner}/${data.repo}/setup-token`,
      {
        headers: authHeaders(idToken),
        method: 'POST',
      },
    )
    const payload = await response.json().catch(() => null)

    if (!response.ok) {
      throw new Error(payload?.error ?? `request failed: ${response.status}`)
    }

    return setupView(api, payload as RepoSetup)
  })

export const Route = createFileRoute('/repos/$owner/$repo/setup')({
  loader: ({ params }) => loadSetupForRequest({ data: params }),
  errorComponent: SetupError,
  component: SetupPage,
})

function SetupPage() {
  const initialSetup = Route.useLoaderData()
  const params = Route.useParams()
  const [setup, setSetup] = useState(initialSetup)
  const [tokenSecret, setTokenSecret] = useState<string | null>(
    initialSetup.token?.secret ?? null,
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const stored = window.sessionStorage.getItem(setupSecretKey(setup.repo.id))
    if (stored) {
      setTokenSecret(stored)
      window.sessionStorage.removeItem(setupSecretKey(setup.repo.id))
    }
  }, [setup.repo.id])

  async function regenerateToken() {
    setBusy(true)
    setError(null)
    try {
      const next = await regenerateTokenForRequest({ data: params })
      setSetup(next)
      setTokenSecret(next.token?.secret ?? null)
    } catch (tokenError) {
      setError(
        tokenError instanceof Error ? tokenError.message : 'token update failed',
      )
    } finally {
      setBusy(false)
    }
  }

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
                {setup.repo.id}
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
              <Badge variant="outline">{setup.repo.lifecycle_state}</Badge>
              <Badge variant="outline">{setup.repo.default_visibility}</Badge>
            </div>
            <h1 className="truncate font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              {setup.repo.id}
            </h1>
          </div>
        </div>

        {error && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Token update failed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <section className="mt-8 border-y border-border">
          <div className="grid gap-4 py-5 md:grid-cols-[180px_minmax(0,1fr)]">
            <div className="flex items-center gap-2 text-sm font-semibold leading-5">
              <KeyRound className="size-4" />
              <span>First-push token</span>
            </div>
            <div className="min-w-0 space-y-3">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{setup.token?.status ?? 'Missing'}</Badge>
                {setup.token && (
                  <span className="text-xs leading-4 text-muted-foreground">
                    Expires {formatUnix(setup.token.expires_at_unix)}
                  </span>
                )}
              </div>
              {tokenSecret ? (
                <code className="block overflow-x-auto rounded-md border border-border bg-muted px-3 py-2 font-mono text-xs leading-5">
                  {tokenSecret}
                </code>
              ) : (
                <Button
                  disabled={busy}
                  onClick={() => void regenerateToken()}
                  size="sm"
                  type="button"
                >
                  {busy ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="size-3.5" />
                  )}
                  <span>Generate token</span>
                </Button>
              )}
            </div>
          </div>
        </section>

        <section className="border-b border-border">
          <div className="grid gap-4 py-5 md:grid-cols-[180px_minmax(0,1fr)]">
            <div className="flex items-center gap-2 text-sm font-semibold leading-5">
              <Terminal className="size-4" />
              <span>Git setup</span>
            </div>
            <div className="min-w-0 space-y-2">
              {!setup.push_enabled && (
                <p className="text-sm leading-5 text-muted-foreground">
                  First-push receive is not enabled in this build yet. These
                  commands are the intended remote and push shape.
                </p>
              )}
              {setup.commands.map((command) => (
                <code
                  className="block overflow-x-auto rounded-md border border-border bg-muted px-3 py-2 font-mono text-xs leading-5"
                  key={command}
                >
                  {command}
                </code>
              ))}
            </div>
          </div>
        </section>
      </section>
    </main>
  )
}

function SetupError({ error }: { error: unknown }) {
  const message =
    error instanceof Error ? error.message : 'Unexpected setup error'

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[720px]">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Setup failed to load</AlertTitle>
          <AlertDescription className="space-y-4">
            <p>{message}</p>
            <Button asChild size="sm" variant="secondary">
              <Link to="/">
                <ArrowLeft className="size-3.5" />
                <span>Repos</span>
              </Link>
            </Button>
          </AlertDescription>
        </Alert>
      </div>
    </main>
  )
}

function parseSetupParams(input: unknown): SetupParams {
  const data = input as Partial<SetupParams> | null
  const owner = typeof data?.owner === 'string' ? data.owner.trim() : ''
  const repo = typeof data?.repo === 'string' ? data.repo.trim() : ''

  if (!owner || !repo) {
    throw new Error('Repository setup route is missing owner or repo.')
  }

  return { owner, repo }
}

async function readRequestAuthToken() {
  const { getCookie } = await import('@tanstack/react-start/server')
  return getCookie(authCookieName)
}

async function loadJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init)
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as T
}

function setupView(api: string, setup: RepoSetup): RepoSetupView {
  const gitRemoteUrl = `${stripTrailingSlash(api)}${setup.git_remote_path}`

  return {
    ...setup,
    commands: [
      `git remote add ${setup.remote_name} ${gitRemoteUrl}`,
      `git push -u ${setup.remote_name} ${setup.push_branch}`,
    ],
    git_remote_url: gitRemoteUrl,
  }
}

function authHeaders(idToken?: string): HeadersInit {
  return idToken ? { authorization: `Bearer ${idToken}` } : {}
}

function formatUnix(value: number) {
  return new Date(value * 1000).toLocaleString()
}

function setupSecretKey(repoId: string) {
  return `scope:first-push-token:${repoId}`
}

function getApiConnection() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error('Set VITE_SCOPE_API_URL before loading repository setup.')
}

function getApiMutationConnection() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error('Set VITE_SCOPE_API_URL before changing repository setup.')
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
