import {
  loadRepoForRequest,
  parseRepoParams,
  parseSetRepoFileVisibilityInput,
  setRepoFileVisibilityForRequest,
} from '@/api/repos'
import type { RepoParams, ReviewFile, Visibility } from '@/api/types'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepo = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoForRequest(data))

const setRepoFileVisibility = createServerFn({ method: 'POST' })
  .validator(parseSetRepoFileVisibilityInput)
  .handler(({ data }) => setRepoFileVisibilityForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/')({
  loader: async ({ params }) => {
    const detail = await loadRepo({ data: params })
    if (detail.review) {
      throw redirect({
        params,
        to: '/repos/$owner/$repo/review',
      })
    }
    return detail
  },
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const detail = Route.useLoaderData()
  const params = Route.useParams()

  return (
    <RepoDetailPage
      detail={detail}
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
