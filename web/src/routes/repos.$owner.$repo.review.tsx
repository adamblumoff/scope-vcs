import {
  applyStagedUpdate,
  loadReviewFileDiff,
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
    const [review, projectionPreviewsResult] = await Promise.all([
      loadReview({ data: params }),
      loadReviewProjectionPreviews({ data: params }).then(
        (projectionPreviews) =>
          ({ projectionPreviews, status: 'fulfilled' }) as const,
        (error) => ({ error, status: 'rejected' }) as const,
      ),
    ])

    if (review.kind === 'NoReview') {
      throw redirect({
        params,
        to: '/repos/$owner/$repo',
      })
    }

    if (projectionPreviewsResult.status === 'rejected') {
      throw projectionPreviewsResult.error
    }

    const { projectionPreviews } = projectionPreviewsResult
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
      loadFileDiff={(data) => loadReviewFileDiff({ data })}
      params={params}
      projectionPreviews={projectionPreviews}
      publishRepo={(data) => publishRepo({ data })}
      rejectStagedUpdate={(data) => rejectStagedUpdate({ data })}
      setReviewVisibility={setReviewVisibility}
    />
  )
}
