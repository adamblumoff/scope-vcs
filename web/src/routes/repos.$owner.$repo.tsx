import {
  loadRepoForRequest,
  parseRepoParams,
  parseSetRepoFileVisibilityInput,
  setRepoFileVisibilityForRequest,
} from '@/api/repos'
import type {
  RepoParams,
  ReviewFile,
  Visibility,
} from '@/api/types'
import {
  RepoDetailError,
  RepoDetailPage,
} from '@/features/repo-detail/repo-detail-page'
import { shouldPollPendingFirstPush } from '@/features/repo-detail/pending-first-push-refresh'
import {
  Outlet,
  createFileRoute,
  redirect,
  useChildMatches,
  useRouter,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useEffect } from 'react'

const PENDING_FIRST_PUSH_POLL_MS = 1500

const loadRepo = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoForRequest(data))

const setRepoFileVisibility = createServerFn({ method: 'POST' })
  .validator(parseSetRepoFileVisibilityInput)
  .handler(({ data }) => setRepoFileVisibilityForRequest(data))

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
  usePendingFirstPushRefresh(
    shouldPollPendingFirstPush(detail, childMatches.length),
  )

  if (childMatches.length > 0) {
    return <Outlet />
  }

  if (!detail) {
    return null
  }

  return (
    <RepoDetailPage
      detail={detail}
      setFileVisibility={setLiveRepoFileVisibility}
      params={params}
    />
  )
}

function usePendingFirstPushRefresh(enabled: boolean) {
  const router = useRouter()

  useEffect(() => {
    if (!enabled) {
      return
    }

    let stopped = false
    let inFlight = false
    const poll = () => {
      if (stopped || inFlight) {
        return
      }
      inFlight = true
      void router
        .invalidate()
        .catch(() => undefined)
        .finally(() => {
          inFlight = false
        })
    }

    poll()
    const interval = window.setInterval(poll, PENDING_FIRST_PUSH_POLL_MS)
    return () => {
      stopped = true
      window.clearInterval(interval)
    }
  }, [enabled, router])
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
