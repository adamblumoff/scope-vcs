import {
  loadRepoForRequest,
  loadRepoLiveStateForRequest,
  parseRepoParams,
  parseSetRepoFileVisibilityInput,
  setRepoFileVisibilityForRequest,
} from '@/api/repos'
import type {
  RepoDetail,
  RepoLiveState,
  RepoParams,
  ReviewFile,
  Visibility,
} from '@/api/types'
import {
  RepoDetailError,
  RepoDetailPage,
} from '@/features/repo-detail/repo-detail-page'
import { useRepoLiveRefresh } from '@/features/repo-detail/repo-live-refresh'
import {
  Outlet,
  createFileRoute,
  redirect,
  useChildMatches,
  useRouter,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const loadRepo = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoForRequest(data))

const loadRepoLiveState = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoLiveStateForRequest(data))

const setRepoFileVisibility = createServerFn({ method: 'POST' })
  .validator(parseSetRepoFileVisibilityInput)
  .handler(({ data }) => setRepoFileVisibilityForRequest(data))

type RepoRouteData =
  | {
      detail: RepoDetail
      kind: 'detail'
      live: RepoLiveState
    }
  | {
      kind: 'live'
      live: RepoLiveState
    }

export const Route = createFileRoute('/repos/$owner/$repo')({
  loader: async ({ location, params }) => {
    if (
      stripTrailingSlash(location.pathname) !==
      `/repos/${params.owner}/${params.repo}`
    ) {
      return {
        kind: 'live',
        live: await loadRepoLiveState({ data: params }),
      } satisfies RepoRouteData
    }

    const detail = await loadRepo({ data: params })
    if (detail.review) {
      throw redirect({
        params,
        to: '/repos/$owner/$repo/review',
      })
    }

    return {
      detail,
      kind: 'detail',
      live: detail.live,
    } satisfies RepoRouteData
  },
  errorComponent: RepoDetailError,
  component: RepoDetailRoute,
})

function stripTrailingSlash(path: string) {
  return path.length > 1 ? path.replace(/\/+$/, '') : path
}

function RepoDetailRoute() {
  const childMatches = useChildMatches()
  const routeData = Route.useLoaderData()
  const params = Route.useParams()
  const router = useRouter()
  const invalidate = useCallback(() => router.invalidate(), [router])
  useRepoLiveRefresh(routeData.live, invalidate)

  if (childMatches.length > 0) {
    return <Outlet />
  }

  if (routeData.kind !== 'detail') {
    return null
  }

  return (
    <RepoDetailPage
      detail={routeData.detail}
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
