import { setupPushSecretKey } from '@/api/setup'
import type {
  RepoLifecycleState,
  RepoParams,
  RepoSetupView,
  SetupProgressState,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link, useNavigate, useRouter } from '@tanstack/react-router'
import {
  AlertCircle,
  ArrowLeft,
  LoaderCircle,
  RefreshCw,
  Terminal,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useState } from 'react'
import { setupCommand } from './commands'

export function SetupPage({
  initialSetup,
  loadProgress,
  params,
  regenerateToken,
}: {
  initialSetup: RepoSetupView
  loadProgress: (params: RepoParams) => Promise<RepoLifecycleState>
  params: RepoParams
  regenerateToken: (params: RepoParams) => Promise<RepoSetupView>
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const [setup, setSetup] = useState(initialSetup)
  const [pushTokenSecret, setPushTokenSecret] = useState<string | null>(
    initialSetup.push_token?.secret ?? null,
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [progressError, setProgressError] = useState<string | null>(null)
  const [progressState, setProgressState] =
    useState<SetupProgressState>('waiting')
  const setupCommandText = pushTokenSecret
    ? setupCommand(setup, pushTokenSecret)
    : null

  useEffect(() => {
    const storedPush = window.sessionStorage.getItem(
      setupPushSecretKey(setup.repo.id),
    )
    if (storedPush) {
      setPushTokenSecret(storedPush)
      window.sessionStorage.removeItem(setupPushSecretKey(setup.repo.id))
    }
  }, [setup.repo.id])

  useEffect(() => {
    let cancelled = false
    let inFlight = false

    async function checkProgress() {
      if (inFlight) {
        return
      }

      inFlight = true
      try {
        const lifecycleState = await loadProgress(params)
        if (cancelled) {
          return
        }

        setProgressError(null)
        if (lifecycleState === 'PendingPublish') {
          setProgressState('opening-review')
          await navigate({
            params,
            replace: true,
            to: '/repos/$owner/$repo/review',
          })
        } else if (lifecycleState === 'Published') {
          setProgressState('published')
          window.sessionStorage.setItem(
            'scope:home-flash',
            `${params.owner}/${params.repo} is published.`,
          )
          await navigate({ replace: true, to: '/' })
          await router.invalidate()
        } else {
          setProgressState('waiting')
        }
      } catch (progressError) {
        if (!cancelled) {
          setProgressError(
            progressError instanceof Error
              ? progressError.message
              : 'setup progress check failed',
          )
        }
      } finally {
        inFlight = false
      }
    }

    const checkOnFocus = () => void checkProgress()
    const checkOnVisibility = () => {
      if (document.visibilityState === 'visible') {
        void checkProgress()
      }
    }
    const initialCheck = window.setTimeout(() => void checkProgress(), 250)
    const interval = window.setInterval(() => void checkProgress(), 1000)
    window.addEventListener('focus', checkOnFocus)
    document.addEventListener('visibilitychange', checkOnVisibility)

    return () => {
      cancelled = true
      window.clearTimeout(initialCheck)
      window.clearInterval(interval)
      window.removeEventListener('focus', checkOnFocus)
      document.removeEventListener('visibilitychange', checkOnVisibility)
    }
  }, [loadProgress, navigate, params, router])

  async function updateToken() {
    setBusy(true)
    setError(null)
    try {
      const next = await regenerateToken(params)
      setSetup(next)
      setPushTokenSecret(next.push_token?.secret ?? null)
    } catch (tokenError) {
      setError(
        tokenError instanceof Error
          ? tokenError.message
          : 'setup command update failed',
      )
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader subtitle={setup.repo.id} subtitleClassName="font-mono" />

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Badge variant="outline">{setup.repo.lifecycle_state}</Badge>
              <VisibilityBadge visibility={setup.repo.default_visibility} />
            </div>
            <h1 className="truncate font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              {setup.repo.id}
            </h1>
            <p className="mt-3 max-w-[640px] text-sm leading-5 text-muted-foreground">
              Run the setup command from your local Git repo. It saves the
              Scope credential, sets the <InlineCode>scope</InlineCode> remote,
              and pushes into review.
            </p>
          </div>
        </div>

        {error && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Setup command update failed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {progressError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Setup status failed</AlertTitle>
            <AlertDescription>{progressError}</AlertDescription>
          </Alert>
        )}

        <section className="mt-8 border-y border-border">
          <div className="grid gap-4 py-5 md:grid-cols-[180px_minmax(0,1fr)]">
            <SectionLabel icon={<Terminal className="size-4" />}>
              Setup command
            </SectionLabel>
            <div className="min-w-0 space-y-2">
              {!setup.push_enabled && (
                <p className="text-sm leading-5 text-muted-foreground">
                  First-push receive is not enabled in this build yet. These
                  commands are the intended remote and push shape.
                </p>
              )}
              <div className="flex items-center gap-2 text-sm leading-5 text-muted-foreground">
                <LoaderCircle className="size-3.5 animate-spin" />
                <span>{setupProgressLabel(progressState)}</span>
              </div>
              {setupCommandText ? (
                <>
                  <CopyableCodeBlock
                    copyLabel="Copy setup command"
                    value={setupCommandText}
                  />
                  <p className="text-sm leading-5 text-muted-foreground">
                    This stores your Scope push token in Git credentials,
                    replaces the local <InlineCode>scope</InlineCode> remote,
                    and pushes your current <InlineCode>HEAD</InlineCode> into
                    review. Your <InlineCode>origin</InlineCode> remote stays
                    untouched.
                  </p>
                </>
              ) : (
                <div className="space-y-3">
                  <p className="text-sm leading-5 text-muted-foreground">
                    Generate a fresh command if the original one was lost or the
                    saved credential expired.
                  </p>
                  <Button
                    disabled={busy}
                    onClick={() => void updateToken()}
                    size="sm"
                    type="button"
                  >
                    {busy ? (
                      <LoaderCircle className="size-3.5 animate-spin" />
                    ) : (
                      <RefreshCw className="size-3.5" />
                    )}
                    <span>Generate setup command</span>
                  </Button>
                </div>
              )}
            </div>
          </div>
        </section>
      </section>
    </main>
  )
}

export function SetupError({ error }: { error: unknown }) {
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

function SectionLabel({
  children,
  icon,
}: {
  children: ReactNode
  icon: ReactNode
}) {
  return (
    <div className="flex items-center gap-2 text-sm font-semibold leading-5">
      {icon}
      <span>{children}</span>
    </div>
  )
}

function InlineCode({ children }: { children: ReactNode }) {
  return (
    <code className="rounded-sm border border-border bg-muted px-1 py-0.5 font-mono text-[0.8em] text-foreground">
      {children}
    </code>
  )
}

function setupProgressLabel(state: SetupProgressState) {
  switch (state) {
    case 'opening-review':
      return 'Upload received. Opening review...'
    case 'published':
      return 'Repo published. Returning home...'
    case 'waiting':
      return 'Watching for your first Git push...'
  }
}
