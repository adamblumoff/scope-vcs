import {
  loadRepoForRequest,
  parseRepoParams,
  parseSetRepoFileVisibilityInput,
  regenerateGitCredentialForRequest,
  setRepoFileVisibilityForRequest,
} from '@/api/repos'
import type { RepoDetail, RepoParams, ReviewFile, Visibility } from '@/api/types'
import { setReviewVisibility } from '@/routes/-repo-review-actions'
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

  return (
    <RepoDetailPage
      detail={detail}
      regenerateGitCredential={(params) =>
        regenerateGitCredential({ data: params })
      }
      setFileVisibility={(params, files, visibility) =>
        setRepoDetailVisibility(params, detail, files, visibility)
      }
      params={params}
    />
  )
}

async function setRepoDetailVisibility(
  params: RepoParams,
  detail: RepoDetail,
  files: ReviewFile[],
  visibility: Visibility,
) {
  if (detail.review) {
    const review = await setReviewVisibility(params, detail.review, files, visibility)
    return review.files
  }

  return setLiveRepoFileVisibility(params, files, visibility)
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
