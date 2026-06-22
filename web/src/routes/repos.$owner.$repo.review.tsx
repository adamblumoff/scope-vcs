import {
  applyStagedUpdate,
  loadReviewProjectionPreviews,
  loadReview,
  publishRepo,
  rejectStagedUpdate,
  setReviewVisibility,
} from '@/routes/-repo-review-actions'
import { ReviewError, ReviewPage } from '@/features/review/review-page'
import { createFileRoute, redirect } from '@tanstack/react-router'

export const Route = createFileRoute('/repos/$owner/$repo/review')({
  loader: async ({ params }) => {
    const review = await loadReview({ data: params })
    if (review.kind === 'NoReview') {
      throw redirect({
        params,
        to: '/repos/$owner/$repo',
      })
    }

    const projectionPreviews = await loadReviewProjectionPreviews({ data: params })
    return { projectionPreviews, review }
  },
  errorComponent: ReviewError,
  component: ReviewRoute,
})

function ReviewRoute() {
  const params = Route.useParams()
  const { projectionPreviews, review } = Route.useLoaderData()

  return (
    <ReviewPage
      applyStagedUpdate={(data) => applyStagedUpdate({ data })}
      initialReview={review}
      params={params}
      projectionPreviews={projectionPreviews}
      publishRepo={(data) => publishRepo({ data })}
      rejectStagedUpdate={(data) => rejectStagedUpdate({ data })}
      setReviewVisibility={setReviewVisibility}
    />
  )
}
