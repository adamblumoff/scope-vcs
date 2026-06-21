import {
  applyStagedUpdate,
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
