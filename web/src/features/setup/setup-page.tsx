import type {
  RepoLifecycleState,
  RepoParams,
  RepoSetupView,
  SetupProgressState,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { PageContent, PageHeader } from '@/components/page-header'
import { RouteErrorPage } from '@/components/route-error-page'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useNavigate, useRouter } from '@tanstack/react-router'
import {
  AlertCircle,
  LoaderCircle,
  RefreshCw,
  Terminal,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useReducer, useSyncExternalStore } from 'react'
import { setupCommand } from './commands'
import {
  rememberSetupPushSecret,
  setupPushSecretSnapshot,
} from './setup-token-cache'

type SetupOverride = {
  baseSetup: RepoSetupView
  pushTokenSecret: string | null
  setup: RepoSetupView
}

type SetupPageState = {
  busy: boolean
  error: string | null
  progressError: string | null
  progressState: SetupProgressState
  setupOverride: SetupOverride | null
}

type SetupPageAction =
  | { message: string; type: 'progressFailed' }
  | { progressState: SetupProgressState; type: 'progressStateChanged' }
  | { type: 'tokenFailed'; message: string }
  | {
      baseSetup: RepoSetupView
      pushTokenSecret: string | null
      setup: RepoSetupView
      type: 'tokenSucceeded'
    }
  | { type: 'tokenStarted' }

const initialSetupPageState: SetupPageState = {
  busy: false,
  error: null,
  progressError: null,
  progressState: 'waiting',
  setupOverride: null,
}

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
  const [state, dispatch] = useReducer(
    setupPageReducer,
    initialSetupPageState,
  )
  const setupOverride =
    state.setupOverride?.baseSetup === initialSetup
      ? state.setupOverride
      : null
  const setup = setupOverride?.setup ?? initialSetup
  const storedPushTokenSecret = useSetupPushSecret(setup.repo.id)
  const pushTokenSecret = setupOverride
    ? setupOverride.pushTokenSecret
    : storedPushTokenSecret ?? setup.push_token?.secret ?? null
  const { busy, error, progressError, progressState } = state
  const setupCommandText = pushTokenSecret
    ? setupCommand(setup, pushTokenSecret)
    : null

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

        if (lifecycleState === 'PendingPublish') {
          dispatch({
            progressState: 'opening-review',
            type: 'progressStateChanged',
          })
          await navigate({
            params,
            replace: true,
            to: '/repos/$owner/$repo/review',
          })
        } else if (lifecycleState === 'Published') {
          dispatch({
            progressState: 'published',
            type: 'progressStateChanged',
          })
          window.sessionStorage.setItem(
            'scope:home-flash',
            `${params.owner}/${params.repo} is published.`,
          )
          await navigate({ replace: true, to: '/' })
          await router.invalidate()
        } else {
          dispatch({
            progressState: 'waiting',
            type: 'progressStateChanged',
          })
        }
      } catch (progressError) {
        if (!cancelled) {
          dispatch({
            message:
              progressError instanceof Error
                ? progressError.message
                : 'setup progress check failed',
            type: 'progressFailed',
          })
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
    dispatch({ type: 'tokenStarted' })
    try {
      const next = await regenerateToken(params)
      const pushTokenSecret = next.push_token?.secret ?? null
      rememberSetupPushSecret(next.repo.id, pushTokenSecret)
      dispatch({
        baseSetup: initialSetup,
        pushTokenSecret,
        setup: next,
        type: 'tokenSucceeded',
      })
    } catch (tokenError) {
      dispatch({
        message:
          tokenError instanceof Error
            ? tokenError.message
            : 'setup command update failed',
        type: 'tokenFailed',
      })
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader subtitle={setup.repo.id} subtitleClassName="font-mono" />

      <PageContent>
        <PageHeader
          badges={() => (
            <>
              <Badge variant="outline">{setup.repo.lifecycle_state}</Badge>
              <VisibilityBadge visibility={setup.repo.default_visibility} />
            </>
          )}
          description={() => (
            <>
              Run the setup command from your local Git repo. It saves the
              Scope credential, sets the <InlineCode>scope</InlineCode> remote,
              and pushes into review.
            </>
          )}
          title={setup.repo.id}
          titleClassName="font-mono"
        />

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

        <SectionRows>
          <SectionRow
            columns="compact"
            icon={<Terminal className="size-4" />}
            title="Setup command"
          >
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
          </SectionRow>
        </SectionRows>
      </PageContent>
    </main>
  )
}

function setupPageReducer(
  state: SetupPageState,
  action: SetupPageAction,
): SetupPageState {
  switch (action.type) {
    case 'progressFailed':
      return { ...state, progressError: action.message }
    case 'progressStateChanged':
      return {
        ...state,
        progressError: null,
        progressState: action.progressState,
      }
    case 'tokenFailed':
      return { ...state, busy: false, error: action.message }
    case 'tokenStarted':
      return { ...state, busy: true, error: null }
    case 'tokenSucceeded':
      return {
        ...state,
        busy: false,
        setupOverride: {
          baseSetup: action.baseSetup,
          pushTokenSecret: action.pushTokenSecret,
          setup: action.setup,
        },
      }
  }
}

function useSetupPushSecret(repoId: string) {
  return useSyncExternalStore(
    subscribeSetupPushSecret,
    () => setupPushSecretSnapshot(repoId),
    getServerSetupPushSecretSnapshot,
  )
}

function subscribeSetupPushSecret() {
  return () => {}
}

function getServerSetupPushSecretSnapshot() {
  return null
}

export function SetupError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected setup error"
      title="Setup failed to load"
    />
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
