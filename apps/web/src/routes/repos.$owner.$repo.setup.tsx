import { CopyableCodeBlock } from '@/components/copyable-code-block'
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
import type { ReactNode } from 'react'
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

type GitPushToken = {
  created_at_unix: number
  secret: string | null
}

type RepoSetup = {
  repo: RepoSummary
  git_remote_path: string
  remote_name: string
  push_branch: string
  push_enabled: boolean
  token: FirstPushToken | null
  push_token: GitPushToken | null
}

type RepoSetupView = RepoSetup & {
  commands: string[]
  git_remote_url: string
}

type RepoSetupCommandSource = RepoSetup & {
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
  const [pushTokenSecret, setPushTokenSecret] = useState<string | null>(
    initialSetup.push_token?.secret ?? null,
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const commands = setupCommands(setup)
  const dualPushCommands = dualRemotePushCommands(setup)
  const credentialHost = gitCredentialHost(setup.git_remote_url)

  useEffect(() => {
    const stored = window.sessionStorage.getItem(setupSecretKey(setup.repo.id))
    if (stored) {
      setTokenSecret(stored)
      window.sessionStorage.removeItem(setupSecretKey(setup.repo.id))
    }
    const storedPush = window.sessionStorage.getItem(
      setupPushSecretKey(setup.repo.id),
    )
    if (storedPush) {
      setPushTokenSecret(storedPush)
      window.sessionStorage.removeItem(setupPushSecretKey(setup.repo.id))
    }
  }, [setup.repo.id])

  async function regenerateToken() {
    setBusy(true)
    setError(null)
    try {
      const next = await regenerateTokenForRequest({ data: params })
      setSetup(next)
      setTokenSecret(next.token?.secret ?? null)
      setPushTokenSecret(next.push_token?.secret ?? null)
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
            <p className="mt-3 max-w-[640px] text-sm leading-5 text-muted-foreground">
              Push from your local Git repo, then review file visibility before
              the repo is published. When Git asks for credentials, use the
              Scope token as the password.
            </p>
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
              <p className="text-sm leading-5 text-muted-foreground">
                Run these commands in the local repo you want to upload.
              </p>
              <CopyableCodeBlock
                copyLabel="Copy Git setup commands"
                value={commands.join('\n')}
              />
            </div>
          </div>
        </section>

        <section className="border-b border-border">
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
                <>
                  <CopyableCodeBlock
                    copyLabel="Copy first-push token"
                    value={tokenSecret}
                  />
                  <p className="text-sm leading-5 text-muted-foreground">
                    Use this as the password for the first Git push. The
                    username can be <InlineCode>scope</InlineCode>; Scope ignores
                    it.
                  </p>
                </>
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
              <KeyRound className="size-4" />
              <span>Git push token</span>
            </div>
            <div className="min-w-0 space-y-3">
              {pushTokenSecret ? (
                <>
                  <CopyableCodeBlock
                    copyLabel="Copy Git push token"
                    value={pushTokenSecret}
                  />
                  <p className="text-sm leading-5 text-muted-foreground">
                    Use this for owner pushes and clones after the repo is
                    published. For the first upload, use the first-push token
                    above.
                  </p>
                </>
              ) : (
                <p className="text-sm leading-5 text-muted-foreground">
                  Visible once when the repository is created.
                </p>
              )}
            </div>
          </div>
        </section>

        <section className="border-b border-border">
          <div className="grid gap-4 py-5 md:grid-cols-[180px_minmax(0,1fr)]">
            <div className="flex items-center gap-2 text-sm font-semibold leading-5">
              <KeyRound className="size-4" />
              <span>Git credentials</span>
            </div>
            <div className="min-w-0 space-y-2">
              <p className="text-sm leading-5 text-muted-foreground">
                When Git prompts, enter any username, for example{' '}
                <InlineCode>scope</InlineCode>, and paste the{' '}
                <InlineCode>first-push token</InlineCode> as the password. Do
                not use your GitHub username or password. If a bad password was
                saved, remove the credential for{' '}
                <InlineCode>{credentialHost}</InlineCode> and retry.
              </p>
            </div>
          </div>
        </section>

        <section className="border-b border-border">
          <div className="grid gap-4 py-5 md:grid-cols-[180px_minmax(0,1fr)]">
            <div className="flex items-center gap-2 text-sm font-semibold leading-5">
              <GitBranch className="size-4" />
              <span>GitHub + Scope</span>
            </div>
            <div className="min-w-0 space-y-2">
              <p className="text-sm leading-5 text-muted-foreground">
                You can keep GitHub as <InlineCode>origin</InlineCode> and add
                Scope as the separate remote above. To make{' '}
                <InlineCode>git push origin</InlineCode> send to both, add both
                push URLs to <InlineCode>origin</InlineCode>. Once push URLs are
                configured, Git uses only that list.
              </p>
              <CopyableCodeBlock
                copyLabel="Copy GitHub and Scope commands"
                value={dualPushCommands.join('\n')}
              />
              <p className="text-sm leading-5 text-muted-foreground">
                Fetch and pull still come from your normal GitHub URL. Scope
                will prompt for the first-push token on the first upload and the
                Git push token after publish.
              </p>
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
    commands: setupCommands({ ...setup, git_remote_url: gitRemoteUrl }),
    git_remote_url: gitRemoteUrl,
  }
}

function setupCommands(setup: RepoSetupCommandSource) {
  return [
    `git remote add ${setup.remote_name} ${setup.git_remote_url}`,
    `git push -u ${setup.remote_name} HEAD:${setup.push_branch}`,
  ]
}

function dualRemotePushCommands(setup: RepoSetupCommandSource) {
  return [
    'git remote get-url origin',
    'git remote set-url --add --push origin <github-remote-url>',
    `git remote set-url --add --push origin ${setup.git_remote_url}`,
    `git push origin HEAD:${setup.push_branch}`,
  ]
}

function InlineCode({ children }: { children: ReactNode }) {
  return (
    <code className="rounded-sm border border-border bg-muted px-1 py-0.5 font-mono text-[0.8em] text-foreground">
      {children}
    </code>
  )
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

function setupPushSecretKey(repoId: string) {
  return `scope:git-push-token:${repoId}`
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

function gitCredentialHost(remoteUrl: string) {
  try {
    return new URL(remoteUrl).host
  } catch {
    return remoteUrl
  }
}
