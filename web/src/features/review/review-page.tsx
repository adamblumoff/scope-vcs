import type {
  ProjectionPreviewAudience,
  ProjectionPreviews,
  RepoAccess,
  RepoParams,
  RepoReview,
  ReviewFileDiff,
  ReviewFileDiffInput,
  ReviewFile,
  ReviewLineDiff,
  Visibility,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { RepoBreadcrumb } from '@/components/repo-breadcrumb'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorPage } from '@/components/route-error-page'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { storeHomeFlash } from '@/lib/home-flash'
import { useNavigate, useRouter } from '@tanstack/react-router'
import { GitBranch, LoaderCircle, Rocket, X } from 'lucide-react'
import { useEffect, useMemo, useReducer, useState } from 'react'
import {
  initialReviewPageState,
  reviewPageReducer,
} from './review-page-state'
import { ReviewPreviewMetrics } from './review-preview-metrics'
import { ReviewVisibilityPanel } from './review-visibility-panel'

export function ReviewPage({
  access,
  applyStagedUpdate,
  initialReview,
  params,
  projectionPreviews,
  publishRepo,
  rejectStagedUpdate,
  loadFileDiff,
  setReviewVisibility,
}: {
  access: RepoAccess
  applyStagedUpdate: (params: RepoParams) => Promise<unknown>
  initialReview: RepoReview
  loadFileDiff: (input: ReviewFileDiffInput) => Promise<ReviewFileDiff>
  params: RepoParams
  projectionPreviews: ProjectionPreviews
  publishRepo: (params: RepoParams) => Promise<unknown>
  rejectStagedUpdate: (params: RepoParams) => Promise<unknown>
  setReviewVisibility: (
    params: RepoParams,
    review: RepoReview,
    files: ReviewFile[],
    visibility: Visibility,
  ) => Promise<RepoReview>
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const [state, dispatch] = useReducer(
    reviewPageReducer,
    initialReviewPageState,
  )
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null)
  const [preferredAudience, setPreferredAudience] =
    useState<ProjectionPreviewAudience>('public')
  const [fileDiffState, setFileDiffState] =
    useState<ReviewFileDiffState>(emptyFileDiffState)
  const review =
    state.reviewOverride?.baseReview === initialReview
      ? state.reviewOverride.review
      : initialReview
  const { error, pendingKey } = state
  const publishing = state.runningAction === 'publish'
  const rejecting = state.runningAction === 'reject'
  const stagedReview = review.kind === 'StagedUpdate'
  const canApplyReview =
    review.kind === 'PendingImport'
      ? access.actor === 'Owner'
      : access.can_apply_changes
  const canChangeVisibility = access.can_change_file_visibility
  const visibilityPending = pendingKey !== null
  const availableAudiences = useMemo(
    () =>
      [
        projectionPreviews.owner ? 'owner' : null,
        projectionPreviews.public ? 'public' : null,
      ].filter(Boolean) as ProjectionPreviewAudience[],
    [projectionPreviews.owner, projectionPreviews.public],
  )
  const audience = availableAudiences.includes(preferredAudience)
    ? preferredAudience
    : availableAudiences[0]
  const preview = audience ? projectionPreviews[audience] : null
  const showPrivateCounts = Boolean(projectionPreviews.owner)
  const reviewRailClassName = selectedFilePath
    ? 'max-w-[1320px] transition-[max-width] duration-300 ease-out'
    : 'max-w-[1040px] transition-[max-width] duration-300 ease-out'

  useEffect(() => {
    if (!selectedFilePath) {
      setFileDiffState(emptyFileDiffState)
      return
    }

    let active = true
    setFileDiffState({ diff: null, error: null, status: 'loading' })
    loadFileDiff({
      owner: params.owner,
      path: selectedFilePath,
      repo: params.repo,
    }).then(
      (diff) => {
        if (active) {
          setFileDiffState({ diff, error: null, status: 'loaded' })
        }
      },
      (error) => {
        if (active) {
          setFileDiffState({
            diff: null,
            error: error instanceof Error ? error.message : 'diff load failed',
            status: 'failed',
          })
        }
      },
    )

    return () => {
      active = false
    }
  }, [loadFileDiff, params.owner, params.repo, selectedFilePath])

  async function setVisibility(
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) {
    const paths = files.map((file) => file.path)
    if (paths.length === 0) {
      return
    }

    dispatch({ pendingKey, type: 'visibilityStarted' })
    try {
      const updated = await setReviewVisibility(params, review, files, visibility)
      dispatch({
        baseReview: initialReview,
        review: updated,
        type: 'visibilitySucceeded',
      })
      await router.invalidate()
    } catch (visibilityError) {
      dispatch({
        message:
          visibilityError instanceof Error
            ? visibilityError.message
            : 'visibility update failed',
        type: 'visibilityFailed',
      })
    } finally {
      dispatch({ type: 'visibilityFinished' })
    }
  }

  async function completeReview() {
    if (visibilityPending) {
      return
    }
    dispatch({ type: 'publishStarted' })
    try {
      if (review.kind === 'StagedUpdate') {
        await applyStagedUpdate(params)
        storeHomeFlash(`${params.owner}/${params.repo} update applied.`)
      } else {
        await publishRepo(params)
        storeHomeFlash(`${params.owner}/${params.repo} published.`)
      }
      await navigate({ replace: true, to: '/' })
      await router.invalidate()
    } catch (publishError) {
      dispatch({
        message:
          publishError instanceof Error
            ? publishError.message
            : 'review action failed',
        type: 'actionFailed',
      })
    }
  }

  async function rejectUpdate() {
    if (visibilityPending) {
      return
    }
    dispatch({ type: 'rejectStarted' })
    try {
      await rejectStagedUpdate(params)
      storeHomeFlash(`${params.owner}/${params.repo} update rejected.`)
      await navigate({ replace: true, to: '/' })
      await router.invalidate()
    } catch (rejectError) {
      dispatch({
        message: rejectError instanceof Error ? rejectError.message : 'reject failed',
        type: 'actionFailed',
      })
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader
        breadcrumb={<RepoBreadcrumb params={params} section="review" />}
        contentClassName={reviewRailClassName}
      />

      <PageContent className={reviewRailClassName}>
        <PageHeader
          actions={() => (
            <>
              {preview && (
                <ReviewPreviewMetrics
                  preview={preview}
                  showPrivateCounts={showPrivateCounts}
                />
              )}
            </>
          )}
          badges={() => (
            <>
              <LifecycleBadge state={review.publication_state} />
              {review.default_visibility && (
                <VisibilityBadge visibility={review.default_visibility} />
              )}
              <Badge variant="neutral">{review.files.length} files</Badge>
              {stagedReview && review.branch && (
                <Badge variant="neutral">
                  <GitBranch className="size-3" />
                  {review.branch}
                </Badge>
              )}
            </>
          )}
          title={`${params.owner}/${params.repo}`}
          titleClassName="font-mono"
        >
          <div className="mt-4 flex flex-wrap items-center gap-2">
            {stagedReview && canApplyReview && (
              <Button
                disabled={
                  publishing ||
                  rejecting ||
                  visibilityPending ||
                  review.files.length === 0
                }
                onClick={() => void rejectUpdate()}
                size="sm"
                variant="danger"
                type="button"
              >
                {rejecting ? (
                  <LoaderCircle className="size-3.5 animate-spin" />
                ) : (
                  <X className="size-3.5" />
                )}
                <span>{rejecting ? 'Rejecting' : 'Reject'}</span>
              </Button>
            )}
            {canApplyReview && (
              <Button
                disabled={
                  publishing ||
                  rejecting ||
                  visibilityPending ||
                  (stagedReview && review.files.length === 0)
                }
                onClick={() => void completeReview()}
                size="sm"
                variant="success"
                type="button"
              >
                {publishing ? (
                  <LoaderCircle className="size-3.5 animate-spin" />
                ) : (
                  <Rocket className="size-3.5" />
                )}
                <span>
                  {publishing
                    ? stagedReview
                      ? 'Applying'
                      : 'Publishing'
                    : stagedReview
                      ? 'Apply'
                      : 'Publish'}
                </span>
              </Button>
            )}
            <ReviewLineDiffPill lineDiff={review.line_diff} />
          </div>
        </PageHeader>

        {error && (
          <PageErrorAlert title="Review update failed">
            {error}
          </PageErrorAlert>
        )}

        <ReviewVisibilityPanel
          disabled={publishing || rejecting}
          files={review.files}
          onCloseFileDiff={() => setSelectedFilePath(null)}
          onSelectFile={(file) =>
            setSelectedFilePath((currentPath) =>
              currentPath === file.path ? null : file.path,
            )
          }
          onSetVisibility={
            canChangeVisibility
              ? (files, visibility, key) =>
                  void setVisibility(files, visibility, key)
              : undefined
          }
          pendingKey={pendingKey}
          preferredAudience={audience}
          previews={projectionPreviews}
          selectedFileDiff={fileDiffState.diff}
          selectedFileDiffError={fileDiffState.error}
          selectedFileDiffLoading={fileDiffState.status === 'loading'}
          selectedFilePath={selectedFilePath}
          onSelectAudience={setPreferredAudience}
          stagedReview={stagedReview}
        />
      </PageContent>
    </main>
  )
}

type ReviewFileDiffState =
  | { diff: null; error: null; status: 'idle' }
  | { diff: null; error: null; status: 'loading' }
  | { diff: ReviewFileDiff; error: null; status: 'loaded' }
  | { diff: null; error: string; status: 'failed' }

const emptyFileDiffState: ReviewFileDiffState = {
  diff: null,
  error: null,
  status: 'idle',
}

function ReviewLineDiffPill({
  lineDiff,
}: {
  lineDiff: ReviewLineDiff | null | undefined
}) {
  if (!lineDiff) {
    return null
  }

  return (
    <div
      aria-label={`${lineDiff.deletions} deletions and ${lineDiff.additions} additions`}
      className="inline-flex items-center gap-2 px-1 font-mono text-base leading-9"
    >
      <span className="text-red-300">-{lineDiff.deletions}</span>
      <span className="text-green-300">+{lineDiff.additions}</span>
    </div>
  )
}

export function ReviewError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected review error"
      title="Review unavailable"
    />
  )
}
