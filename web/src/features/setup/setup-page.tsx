import type {
  RepoLifecycleState,
  RepoParams,
  RepoSetupView,
  SetupProgressState,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorPage } from '@/components/route-error-page'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useNavigate, useRouter } from '@tanstack/react-router'
import { LoaderCircle, RefreshCw, Terminal } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCallback, useReducer } from 'react'
import { setupCommand } from './commands'
import {
  initialSetupPageState,
  setupPageReducer,
} from './setup-page-state'
import { useSetupProgress } from './use-setup-progress'
import { useRegenerateSetupToken, useSetupPushSecret } from './use-setup-token'

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
  const updateToken = useRegenerateSetupToken({
    dispatch,
    initialSetup,
    params,
    regenerateToken,
  })
  const updateProgressState = useCallback(
    (nextProgressState: SetupProgressState) =>
      dispatch({
        progressState: nextProgressState,
        type: 'progressStateChanged',
      }),
    [],
  )
  const updateProgressError = useCallback(
    (message: string) => dispatch({ message, type: 'progressFailed' }),
    [],
  )
  const openReview = useCallback(
    () =>
      navigate({
        params,
        replace: true,
        to: '/repos/$owner/$repo/review',
      }),
    [navigate, params],
  )
  const returnHomeAfterPublish = useCallback(async () => {
    window.sessionStorage.setItem(
      'scope:home-flash',
      `${params.owner}/${params.repo} is published.`,
    )
    await navigate({ replace: true, to: '/' })
    await router.invalidate()
  }, [navigate, params, router])

  useSetupProgress({
    loadProgress,
    onProgressError: updateProgressError,
    onProgressState: updateProgressState,
    onPublished: returnHomeAfterPublish,
    onReviewReady: openReview,
    params,
  })

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
          <PageErrorAlert title="Setup command update failed">
            {error}
          </PageErrorAlert>
        )}

        {progressError && (
          <PageErrorAlert title="Setup status failed">
            {progressError}
          </PageErrorAlert>
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
