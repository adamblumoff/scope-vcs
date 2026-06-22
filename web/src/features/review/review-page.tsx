import { storeHomeFlash } from '@/api/client'
import type {
  ProjectionPreviews,
  RepoParams,
  RepoReview,
  ReviewFile,
  Visibility,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link, useNavigate, useRouter } from '@tanstack/react-router'
import { AlertCircle, ArrowLeft, LoaderCircle, Rocket, X } from 'lucide-react'
import { useReducer } from 'react'
import { ReviewVisibilityPanel } from './review-visibility-panel'

type ReviewOverride = {
  baseReview: RepoReview
  review: RepoReview
}

type ReviewPageState = {
  error: string | null
  pendingKey: string | null
  reviewOverride: ReviewOverride | null
  runningAction: 'publish' | 'reject' | null
}

type ReviewPageAction =
  | { type: 'actionFailed'; message: string }
  | { type: 'publishStarted' }
  | { type: 'rejectStarted' }
  | { type: 'visibilityFailed'; message: string }
  | { type: 'visibilityFinished' }
  | { type: 'visibilityStarted'; pendingKey: string }
  | { baseReview: RepoReview; review: RepoReview; type: 'visibilitySucceeded' }

const initialReviewPageState: ReviewPageState = {
  error: null,
  pendingKey: null,
  reviewOverride: null,
  runningAction: null,
}

export function ReviewPage({
  applyStagedUpdate,
  initialReview,
  params,
  projectionPreviews,
  publishRepo,
  rejectStagedUpdate,
  setReviewVisibility,
}: {
  applyStagedUpdate: (params: RepoParams) => Promise<unknown>
  initialReview: RepoReview
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
  const review =
    state.reviewOverride?.baseReview === initialReview
      ? state.reviewOverride.review
      : initialReview
  const { error, pendingKey } = state
  const publishing = state.runningAction === 'publish'
  const rejecting = state.runningAction === 'reject'
  const stagedReview = review.kind === 'StagedUpdate'
  const visibilityPending = pendingKey !== null

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

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Badge variant="outline">{review.publication_state}</Badge>
              {review.default_visibility && (
                <VisibilityBadge visibility={review.default_visibility} />
              )}
              <Badge variant="outline">{review.files.length} files</Badge>
              {stagedReview && review.branch && (
                <Badge variant="outline">{review.branch}</Badge>
              )}
            </div>
            <h1 className="truncate font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              {params.owner}/{params.repo}
            </h1>
          </div>
          <div className="flex items-center gap-2">
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
          </div>
        </div>

        {error && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Review update failed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <ReviewVisibilityPanel
          disabled={publishing || rejecting}
          files={review.files}
          onSetVisibility={(files, visibility, key) =>
            void setVisibility(files, visibility, key)
          }
          pendingKey={pendingKey}
          previews={projectionPreviews}
          stagedReview={stagedReview}
        />
      </section>
    </main>
  )
}

function reviewPageReducer(
  state: ReviewPageState,
  action: ReviewPageAction,
): ReviewPageState {
  switch (action.type) {
    case 'actionFailed':
      return { ...state, error: action.message, runningAction: null }
    case 'publishStarted':
      return { ...state, error: null, runningAction: 'publish' }
    case 'rejectStarted':
      return { ...state, error: null, runningAction: 'reject' }
    case 'visibilityFailed':
      return { ...state, error: action.message }
    case 'visibilityFinished':
      return { ...state, pendingKey: null }
    case 'visibilityStarted':
      return { ...state, error: null, pendingKey: action.pendingKey }
    case 'visibilitySucceeded':
      return {
        ...state,
        reviewOverride: {
          baseReview: action.baseReview,
          review: action.review,
        },
      }
  }
}

export function ReviewError({ error }: { error: unknown }) {
  const message =
    error instanceof Error ? error.message : 'Unexpected review error'

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[760px] border-y border-border py-6">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Review unavailable</AlertTitle>
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
