import {
  loadRepoForRequest,
  parseRepoParams,
  parseSetRepoFileVisibilityInput,
  createCloneCredentialForRequest,
  setRepoFileVisibilityForRequest,
} from '@/api/repos'
import type {
  RepoCloneCredentialView,
  RepoParams,
  ReviewFile,
  Visibility,
} from '@/api/types'
import {
  RepoDetailError,
  RepoDetailPage,
} from '@/features/repo-detail/repo-detail-page'
import {
  Outlet,
  createFileRoute,
  redirect,
  useChildMatches,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepo = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoForRequest(data))

const setRepoFileVisibility = createServerFn({ method: 'POST' })
  .validator(parseSetRepoFileVisibilityInput)
  .handler(({ data }) => setRepoFileVisibilityForRequest(data))

const createCloneCredential = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => createCloneCredentialForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo')({
  loader: async ({ location, params }) => {
    if (
      stripTrailingSlash(location.pathname) !==
      `/repos/${params.owner}/${params.repo}`
    ) {
      return null
    }

    const detail = await loadRepo({ data: params })
    if (detail.review) {
      throw redirect({
        params,
        to: '/repos/$owner/$repo/review',
      })
    }

    return detail
  },
  errorComponent: RepoDetailError,
  component: RepoDetailRoute,
})

function stripTrailingSlash(path: string) {
  return path.length > 1 ? path.replace(/\/+$/, '') : path
}

function RepoDetailRoute() {
  const childMatches = useChildMatches()
  const detail = Route.useLoaderData()
  const params = Route.useParams()

  if (childMatches.length > 0) {
    return <Outlet />
  }

  if (!detail) {
    return null
  }

  return (
    <RepoDetailPage
      detail={detail}
      loadCloneCredential={loadCloneCredential}
      setFileVisibility={setLiveRepoFileVisibility}
      params={params}
    />
  )
}

function loadCloneCredential(
  params: RepoParams,
): Promise<RepoCloneCredentialView> {
  return createCloneCredential({ data: params })
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
