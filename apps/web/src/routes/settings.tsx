import {
  Alert,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { authCookieName } from '@/lib/auth'
import { cn } from '@/lib/utils'
import { Link, createFileRoute } from '@tanstack/react-router'
import type { ErrorComponentProps } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  AlertCircle,
  ArrowLeft,
  GitBranch,
  LoaderCircle,
} from 'lucide-react'
import { useState } from 'react'

type RepoSettings = {
  include_ignored_files: boolean
}

type RepoSettingsInput = {
  include_ignored_files: boolean
}

const repoOwner = 'adamblumoff'
const repoName = 'scope-vcs'
const repoId = `${repoOwner}/${repoName}`
const localApiBase = 'http://localhost:8080'

const loadSettingsForRequest = createServerFn({ method: 'GET' }).handler(
  async () => {
    const idToken = await readRequestAuthToken()

    if (!idToken) {
      throw new Error('Sign in as the repo owner to edit settings.')
    }

    return loadRepoSettings(idToken)
  },
)

const updateSettingsForRequest = createServerFn({ method: 'POST' })
  .validator(parseRepoSettingsInput)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()

    if (!idToken) {
      throw new Error('Sign in as the repo owner to edit settings.')
    }

    const api = getApiMutationConnection()
    const response = await fetch(
      `${api}/v1/repos/${repoOwner}/${repoName}/settings`,
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

    return payload as RepoSettings
  })

export const Route = createFileRoute('/settings')({
  loader: () => loadSettingsForRequest(),
  errorComponent: SettingsError,
  component: SettingsPage,
})

function SettingsPage() {
  const initialSettings = Route.useLoaderData()
  const [settings, setSettings] = useState(initialSettings)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function toggleIgnoredFiles() {
    const next = {
      include_ignored_files: !settings.include_ignored_files,
    }

    setBusy(true)
    setError(null)

    try {
      setSettings(await updateSettingsForRequest({ data: next }))
    } catch (updateError) {
      setError(
        updateError instanceof Error
          ? updateError.message
          : 'settings update failed',
      )
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <SettingsHeader />

      <section className="mx-auto max-w-[860px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="border-b border-border pb-6">
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <Badge variant="outline">Owner</Badge>
            <Badge variant="outline">{repoId}</Badge>
          </div>
          <h1 className="font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
            Settings
          </h1>
        </div>

        {error && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Settings update failed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <section className="mt-8 border-y border-border">
          <div className="flex flex-col gap-4 py-4 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <h2 className="text-sm font-semibold leading-5">Ignored files</h2>
              <p className="mt-1 text-sm leading-5 text-muted-foreground">
                Include files matched by Git ignore rules.
              </p>
            </div>
            <SettingsSwitch
              checked={settings.include_ignored_files}
              disabled={busy}
              label="Include ignored files"
              onClick={() => void toggleIgnoredFiles()}
            />
          </div>
        </section>
      </section>
    </main>
  )
}

function SettingsHeader() {
  return (
    <header className="border-b border-border bg-background">
      <div className="mx-auto flex min-h-16 max-w-[860px] items-center justify-between gap-3 px-4 py-3 sm:px-6">
        <Link className="flex min-w-0 items-center gap-3" to="/">
          <div className="flex size-8 shrink-0 items-center justify-center rounded-md border border-border">
            <GitBranch className="size-4" />
          </div>
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold leading-5">Scope</div>
            <div className="truncate font-mono text-xs leading-4 text-muted-foreground">
              {repoId}
            </div>
          </div>
        </Link>
        <Button asChild size="sm" variant="secondary">
          <Link to="/">
            <ArrowLeft className="size-3.5" />
            <span>Back</span>
          </Link>
        </Button>
      </div>
    </header>
  )
}

function SettingsSwitch({
  checked,
  disabled,
  label,
  onClick,
}: {
  checked: boolean
  disabled: boolean
  label: string
  onClick: () => void
}) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={cn(
        'inline-flex h-8 w-[88px] items-center justify-between rounded-full border px-1 text-xs font-medium transition-colors',
        checked
          ? 'border-green-400 bg-green-100 text-green-900'
          : 'border-border bg-muted text-muted-foreground',
        disabled && 'cursor-not-allowed opacity-55',
      )}
      disabled={disabled}
      onClick={onClick}
      role="switch"
      type="button"
    >
      <span className={cn('px-2', !checked && 'order-2')}>
        {disabled ? (
          <LoaderCircle className="size-3.5 animate-spin" />
        ) : checked ? (
          'On'
        ) : (
          'Off'
        )}
      </span>
      <span className="size-6 rounded-full bg-background shadow-sm" />
    </button>
  )
}

function SettingsError({ error }: ErrorComponentProps) {
  const message =
    error instanceof Error ? error.message : 'Unexpected settings error'

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[720px]">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Settings failed to load</AlertTitle>
          <AlertDescription className="space-y-4">
            <p>{message}</p>
            <Button asChild size="sm" variant="secondary">
              <Link to="/">
                <ArrowLeft className="size-3.5" />
                <span>Back</span>
              </Link>
            </Button>
          </AlertDescription>
        </Alert>
      </div>
    </main>
  )
}

async function loadRepoSettings(idToken: string) {
  return loadJson<RepoSettings>(
    `${getApiConnection()}/v1/repos/${repoOwner}/${repoName}/settings`,
    { headers: authHeaders(idToken) },
  )
}

function parseRepoSettingsInput(input: unknown): RepoSettingsInput {
  const data = input as Partial<RepoSettingsInput> | null

  return {
    include_ignored_files: data?.include_ignored_files === true,
  }
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

function authHeaders(idToken?: string): HeadersInit {
  return idToken ? { authorization: `Bearer ${idToken}` } : {}
}

function getApiConnection() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error('Set VITE_SCOPE_API_URL before loading repository settings.')
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
