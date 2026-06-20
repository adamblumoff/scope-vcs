import { parseRepoParams } from '@/api/repos'
import {
  loadReviewForRequest,
  parseSetVisibilityInput,
  postStagedUpdateAction,
  publishRepoForRequest,
  setVisibilityForRequest,
} from '@/api/review'
import type { RepoReview, ReviewFile, Visibility } from '@/api/types'
import { ReviewError, ReviewPage } from '@/features/review/review-page'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadReview = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadReviewForRequest(data))

const setVisibility = createServerFn({ method: 'POST' })
  .validator(parseSetVisibilityInput)
  .handler(({ data }) => setVisibilityForRequest(data))

const publishRepo = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => publishRepoForRequest(data))

const applyStagedUpdate = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => postStagedUpdateAction(data, 'apply'))

const rejectStagedUpdate = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => postStagedUpdateAction(data, 'reject'))

export const Route = createFileRoute('/repos/$owner/$repo/review')({
  loader: async ({ params }) => {
    const review = await loadReview({ data: params })
    if (review.kind === 'NoReview') {
      throw redirect({
        params,
        to: '/repos/$owner/$repo',
      })
    }

    return review
  },
  errorComponent: ReviewError,
  component: ReviewRoute,
})

function ReviewRoute() {
  const params = Route.useParams()

  return (
    <ReviewPage
      applyStagedUpdate={(data) => applyStagedUpdate({ data })}
      initialReview={Route.useLoaderData()}
      params={params}
      publishRepo={(data) => publishRepo({ data })}
      rejectStagedUpdate={(data) => rejectStagedUpdate({ data })}
      setReviewVisibility={setReviewVisibility}
    />
  )
}

function setReviewVisibility(
  params: { owner: string; repo: string },
  review: RepoReview,
  files: ReviewFile[],
  visibility: Visibility,
) {
  return setVisibility({
    data: {
      ...params,
      kind: review.kind,
      paths: files.map((file) => file.path),
      visibility,
    },
  })
}
