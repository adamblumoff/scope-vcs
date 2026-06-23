import type {
  ProjectionPreviews,
  RepoParams,
  RepoReview,
  ReviewFileDiff,
  ReviewFileDiffInput,
  ReviewFile,
  Visibility,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorPage } from '@/components/route-error-page'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { storeHomeFlash } from '@/lib/home-flash'
import { useNavigate, useRouter } from '@tanstack/react-router'
import { LoaderCircle, Rocket, X } from 'lucide-react'
import { useEffect, useReducer, useState } from 'react'
import {
  initialReviewPageState,
  reviewPageReducer,
} from './review-page-state'
import { ReviewVisibilityPanel } from './review-visibility-panel'

export function ReviewPage({
  applyStagedUpdate,
  initialReview,
  params,
  projectionPreviews,
  publishRepo,
  rejectStagedUpdate,
  loadFileDiff,
  setReviewVisibility,
}: {
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
  const visibilityPending = pendingKey !== null

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
      <AppHeader subtitle={`${params.owner}/${params.repo}`} subtitleClassName="font-mono" />

      <PageContent className="max-w-[1320px]">
        <PageHeader
          actions={() => (
            <>
              {stagedReview && (
                <Button
                  disabled={
                    publishing ||
                    rejecting ||
                    visibilityPending ||
                    review.files.length === 0
                  }
                  onClick={() => void rejectUpdate()}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  {rejecting ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : (
                    <X className="size-3.5" />
                  )}
                  <span>{rejecting ? 'Rejecting' : 'Reject'}</span>
                </Button>
              )}
              <Button
                disabled={
                  publishing ||
                  rejecting ||
                  visibilityPending ||
                  (stagedReview && review.files.length === 0)
                }
                onClick={() => void completeReview()}
                size="sm"
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
            </>
          )}
          badges={() => (
            <>
              <Badge variant="outline">{review.publication_state}</Badge>
              {review.default_visibility && (
                <VisibilityBadge visibility={review.default_visibility} />
              )}
              <Badge variant="outline">{review.files.length} files</Badge>
              {stagedReview && review.branch && (
                <Badge variant="outline">{review.branch}</Badge>
              )}
            </>
          )}
          title={`${params.owner}/${params.repo}`}
          titleClassName="font-mono"
        />

        {error && (
          <PageErrorAlert title="Review update failed">
            {error}
          </PageErrorAlert>
        )}

        <ReviewVisibilityPanel
          disabled={publishing || rejecting}
          files={review.files}
          onCloseFileDiff={() => setSelectedFilePath(null)}
          onSelectFile={(file) => setSelectedFilePath(file.path)}
          onSetVisibility={(files, visibility, key) =>
            void setVisibility(files, visibility, key)
          }
          pendingKey={pendingKey}
          previews={projectionPreviews}
          selectedFileDiff={fileDiffState.diff}
          selectedFileDiffError={fileDiffState.error}
          selectedFileDiffLoading={fileDiffState.status === 'loading'}
          selectedFilePath={selectedFilePath}
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

export function ReviewError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected review error"
      title="Review unavailable"
    />
  )
}
