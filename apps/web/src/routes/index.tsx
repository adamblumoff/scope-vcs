import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { authCookieName, createScopeShooAuth } from '@/lib/auth'
import { cn } from '@/lib/utils'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  AlertCircle,
  ArrowRight,
  GitBranch,
  Globe2,
  LoaderCircle,
  Lock,
  LogIn,
  LogOut,
  Moon,
  Plus,
  Sun,
} from 'lucide-react'
import type { FormEvent } from 'react'
import { useState } from 'react'

type Visibility = 'Private' | 'Public'
type RepoRole = 'Reader' | 'Writer' | 'Maintainer' | 'Owner'
type RepoLifecycleState = 'PendingFirstPush' | 'PendingPublish' | 'Published'

type AccountSession = {
  identity: {
    pairwise_sub: string
    email: string | null
    email_verified: boolean
  } | null
  user: {
    id: string
    handle: string
    email: string
    email_verified: boolean
  } | null
}

type RepoSummary = {
  id: string
  owner_handle: string
  name: string
  lifecycle_state: RepoLifecycleState
  default_visibility: Visibility
  role: RepoRole
}

type FirstPushToken = {
  status: 'Active' | 'Expired' | 'Used'
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
  token: FirstPushToken | null
}

type CreateRepoResponse = {
  repo: RepoSummary
  setup: RepoSetup
}

type HomeState = {
  account: AccountSession | null
  error: string | null
  repositories: RepoSummary[]
  signedIn: boolean
}

type CreateRepoInput = {
  name: string
  visibility: Visibility
}

type ThemeMode = 'dark' | 'light'

const localApiBase = 'http://localhost:8080'

const loadHomeForRequest = createServerFn({ method: 'GET' }).handler(
  async (): Promise<HomeState> => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      return {
        account: null,
        error: null,
        repositories: [],
        signedIn: false,
      }
    }

    try {
      const api = getApiConnection()
      const init = { headers: authHeaders(idToken) }
      const [account, repositories] = await Promise.all([
        loadJson<AccountSession>(`${api}/v1/session`, init),
        loadJson<RepoSummary[]>(`${api}/v1/repos`, init),
      ])

      return {
        account,
        error: null,
        repositories,
        signedIn: true,
      }
    } catch (error) {
      if (error instanceof HttpError && error.status === 401) {
        const { deleteCookie } = await import('@tanstack/react-start/server')
        deleteCookie(authCookieName, { path: '/' })
        return {
          account: null,
          error: null,
          repositories: [],
          signedIn: false,
        }
      }

      return {
        account: null,
        error: error instanceof Error ? error.message : 'request failed',
        repositories: [],
        signedIn: true,
      }
    }
  },
)

const createRepoForRequest = createServerFn({ method: 'POST' })
  .validator(parseCreateRepoInput)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      throw new Error('Sign in to create a repository.')
    }

    const response = await fetch(`${getApiMutationConnection()}/v1/repos`, {
      body: JSON.stringify(data),
      headers: {
        ...authHeaders(idToken),
        'content-type': 'application/json',
      },
      method: 'POST',
    })
    const payload = await response.json().catch(() => null)

    if (!response.ok) {
      throw new Error(payload?.error ?? `request failed: ${response.status}`)
    }

    return payload as CreateRepoResponse
  })

export const Route = createFileRoute('/')({
  loader: () => loadHomeForRequest(),
  component: ScopeHome,
})

function ScopeHome() {
  const home = Route.useLoaderData()
  const navigate = useNavigate()
  const [account, setAccount] = useState(home.account)
  const [createError, setCreateError] = useState<string | null>(null)
  const [repositories, setRepositories] = useState(home.repositories)
  const [sessionError, setSessionError] = useState<string | null>(null)
  const [signedIn, setSignedIn] = useState(home.signedIn)
  const [theme, setTheme] = useState<ThemeMode>('dark')

  function toggleTheme() {
    const nextTheme = theme === 'dark' ? 'light' : 'dark'
    setTheme(nextTheme)
    applyTheme(nextTheme)
  }

  async function createRepository(input: CreateRepoInput) {
    setCreateError(null)
    try {
      const created = await createRepoForRequest({ data: input })
      const repo = created.repo
      setRepositories((current) => [repo, ...current])
      if (created.setup.token?.secret) {
        storeSetupSecret(repo.id, created.setup.token.secret)
      }
      await navigate({
        to: '/repos/$owner/$repo/setup',
        params: { owner: repo.owner_handle, repo: repo.name },
      })
    } catch (error) {
      setCreateError(
        error instanceof Error ? error.message : 'repository creation failed',
      )
    }
  }

  async function signOut() {
    setSessionError(null)
    let response: Response
    try {
      response = await fetch('/auth/session', { method: 'DELETE' })
    } catch (error) {
      setSessionError(error instanceof Error ? error.message : 'sign out failed')
      return
    }

    if (!response.ok) {
      setSessionError(`sign out failed: ${response.status}`)
      return
    }

    createScopeShooAuth().clearIdentity()
    setAccount(null)
    setRepositories([])
    setSignedIn(false)
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <header className="border-b border-border bg-background">
        <div className="mx-auto flex min-h-16 max-w-[980px] items-center justify-between gap-3 px-4 py-3 sm:px-6">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-md border border-border">
              <GitBranch className="size-4" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold leading-5">Scope</div>
              <div className="truncate text-xs leading-4 text-muted-foreground">
                {account?.user?.handle ?? 'Repositories'}
              </div>
            </div>
          </div>

          <div className="flex min-w-0 items-center gap-2">
            <AuthControls signedIn={signedIn} signOut={signOut} />
            <ThemeToggle theme={theme} toggleTheme={toggleTheme} />
          </div>
        </div>
      </header>

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <h1 className="text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              Repositories
            </h1>
            {account?.identity && (
              <div className="mt-2 truncate text-sm leading-5 text-muted-foreground">
                {account.identity.email ?? account.identity.pairwise_sub}
              </div>
            )}
          </div>
          {signedIn && <CreateRepoForm onCreate={createRepository} />}
        </div>

        {home.error && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Repositories unavailable</AlertTitle>
            <AlertDescription>{home.error}</AlertDescription>
          </Alert>
        )}

        {createError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Repository creation failed</AlertTitle>
            <AlertDescription>{createError}</AlertDescription>
          </Alert>
        )}

        {sessionError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Session update failed</AlertTitle>
            <AlertDescription>{sessionError}</AlertDescription>
          </Alert>
        )}

        <RepoList repositories={repositories} signedIn={signedIn} />
      </section>
    </main>
  )
}

function CreateRepoForm({
  onCreate,
}: {
  onCreate: (input: CreateRepoInput) => Promise<void>
}) {
  const [busy, setBusy] = useState(false)
  const [name, setName] = useState('')
  const [visibility, setVisibility] = useState<Visibility>('Private')

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!name.trim()) {
      return
    }

    setBusy(true)
    try {
      await onCreate({ name, visibility })
      setName('')
      setVisibility('Private')
    } finally {
      setBusy(false)
    }
  }

  return (
    <form
      className="grid w-full gap-2 sm:w-auto sm:grid-cols-[180px_120px_auto]"
      onSubmit={(event) => void submit(event)}
    >
      <input
        aria-label="Repository name"
        className="h-9 min-w-0 rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        disabled={busy}
        onChange={(event) => setName(event.target.value)}
        placeholder="new-repo"
        value={name}
      />
      <select
        aria-label="Default visibility"
        className="h-9 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        disabled={busy}
        onChange={(event) => setVisibility(event.target.value as Visibility)}
        value={visibility}
      >
        <option value="Private">Private</option>
        <option value="Public">Public</option>
      </select>
      <Button disabled={busy || !name.trim()} size="sm" type="submit">
        {busy ? (
          <LoaderCircle className="size-3.5 animate-spin" />
        ) : (
          <Plus className="size-3.5" />
        )}
        <span>Create</span>
      </Button>
    </form>
  )
}

function RepoList({
  repositories,
  signedIn,
}: {
  repositories: RepoSummary[]
  signedIn: boolean
}) {
  if (repositories.length === 0) {
    return (
      <div className="mt-8 border-y border-border">
        <div className="grid gap-2 py-10 text-sm sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
          <div className="min-w-0">
            <div className="font-medium leading-5">No repositories</div>
            <div className="mt-1 leading-5 text-muted-foreground">
              {signedIn
                ? 'Create a repository to start.'
                : 'Sign in to start from an empty workspace.'}
            </div>
          </div>
          {!signedIn && (
            <Button size="sm" onClick={() => void signIn()} type="button">
              <LogIn className="size-3.5" />
              <span>Sign in</span>
            </Button>
          )}
        </div>
      </div>
    )
  }

  return (
    <div className="mt-8 divide-y divide-border border-y border-border">
      {repositories.map((repo) => (
        <div
          className="grid gap-3 py-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
          key={repo.id}
        >
          <div className="min-w-0">
            <div className="truncate font-mono text-sm font-semibold leading-5">
              {repo.id}
            </div>
            <div className="mt-1 flex flex-wrap gap-2 text-xs leading-4 text-muted-foreground">
              <span>{lifecycleLabel(repo.lifecycle_state)}</span>
              <span>{repo.role}</span>
            </div>
          </div>
          <div className="flex items-center gap-2 sm:justify-end">
            <VisibilityBadge visibility={repo.default_visibility} />
            {repo.lifecycle_state === 'PendingFirstPush' && (
              <Button asChild size="sm" variant="secondary">
                <Link
                  params={{ owner: repo.owner_handle, repo: repo.name }}
                  to="/repos/$owner/$repo/setup"
                >
                  <ArrowRight className="size-3.5" />
                  <span>Setup</span>
                </Link>
              </Button>
            )}
            {repo.lifecycle_state === 'PendingPublish' && (
              <Button asChild size="sm" variant="secondary">
                <Link
                  params={{ owner: repo.owner_handle, repo: repo.name }}
                  to="/repos/$owner/$repo/review"
                >
                  <ArrowRight className="size-3.5" />
                  <span>Review</span>
                </Link>
              </Button>
            )}
          </div>
        </div>
      ))}
    </div>
  )
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
      type="button"
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
  signedIn,
  signOut,
}: {
  signedIn: boolean
  signOut: () => Promise<void>
}) {
  const [busy, setBusy] = useState(false)
  const title = signedIn ? 'Sign out' : 'Sign in with Shoo'

  async function toggleAuth() {
    setBusy(true)

    if (signedIn) {
      await signOut()
      setBusy(false)
      return
    }

    try {
      await signIn()
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
      type="button"
      variant={signedIn ? 'secondary' : 'default'}
    >
      {busy ? (
        <LoaderCircle className="size-3.5 animate-spin" />
      ) : signedIn ? (
        <LogOut className="size-3.5" />
      ) : (
        <LogIn className="size-3.5" />
      )}
      {!busy && (
        <span className="hidden sm:inline">{signedIn ? 'Sign out' : 'Sign in'}</span>
      )}
    </Button>
  )
}

async function signIn() {
  await createScopeShooAuth().startSignIn({ requestPii: true })
}

async function readRequestAuthToken() {
  const { getCookie } = await import('@tanstack/react-start/server')
  return getCookie(authCookieName)
}

function parseCreateRepoInput(input: unknown): CreateRepoInput {
  const data = input as Partial<CreateRepoInput> | null
  const name = typeof data?.name === 'string' ? data.name.trim() : ''
  const visibility = data?.visibility === 'Public' ? 'Public' : 'Private'

  if (!name) {
    throw new Error('Repository name is required.')
  }

  return { name, visibility }
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

function storeSetupSecret(repoId: string, secret: string) {
  if (typeof window === 'undefined') {
    return
  }

  window.sessionStorage.setItem(setupSecretKey(repoId), secret)
}

export function setupSecretKey(repoId: string) {
  return `scope:first-push-token:${repoId}`
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

  throw new Error('Set VITE_SCOPE_API_URL before loading repositories.')
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
