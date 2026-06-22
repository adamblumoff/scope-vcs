import {
  loadRepoForRequest,
  parseRepoParams,
  parseSetRepoFileVisibilityInput,
  regenerateGitCredentialForRequest,
  setRepoFileVisibilityForRequest,
} from '@/api/repos'
import type { RepoParams, ReviewFile, Visibility } from '@/api/types'
import {
  applyStagedUpdate,
  publishRepo,
  rejectStagedUpdate,
  setReviewVisibility,
} from '@/routes/-repo-review-actions'
import { ReviewPage } from '@/features/review/review-page'
import {
  RepoDetailError,
  RepoDetailPage,
} from '@/features/repo-detail/repo-detail-page'
import { Outlet, createFileRoute, useChildMatches } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepo = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoForRequest(data))

const setRepoFileVisibility = createServerFn({ method: 'POST' })
  .validator(parseSetRepoFileVisibilityInput)
  .handler(({ data }) => setRepoFileVisibilityForRequest(data))

const regenerateGitCredential = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => regenerateGitCredentialForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo')({
  loader: ({ params }) => loadRepo({ data: params }),
  errorComponent: RepoDetailError,
  component: RepoDetailRoute,
})

function RepoDetailRoute() {
  const childMatches = useChildMatches()
  const detail = Route.useLoaderData()
  const params = Route.useParams()

  if (childMatches.length > 0) {
    return <Outlet />
  }

  if (detail.review) {
    return (
      <ReviewPage
        applyStagedUpdate={(data) => applyStagedUpdate({ data })}
        initialReview={detail.review}
        params={params}
        projectionPreviews={detail.projection_previews}
        publishRepo={(data) => publishRepo({ data })}
        rejectStagedUpdate={(data) => rejectStagedUpdate({ data })}
        setReviewVisibility={setReviewVisibility}
      />
    )
  }

  return (
    <RepoDetailPage
      detail={detail}
      regenerateGitCredential={(params) =>
        regenerateGitCredential({ data: params })
      }
      setFileVisibility={setLiveRepoFileVisibility}
      params={params}
    />
  )
}

function setLiveRepoFileVisibility(
  params: RepoParams,
  files: ReviewFile[],
  visibility: Visibility,
) {
  return setRepoFileVisibility({
    data: {
      ...params,
      paths: files.map((file) => file.path),
      visibility,
    },
  })
}
