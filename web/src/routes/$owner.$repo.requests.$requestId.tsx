import { createApiClient, HttpError } from '@/api/client'
import type { AccountSession } from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import { loadRequestForRequest } from '@/api/repos'
import {
  loadRequestRatingsForRequest,
  rateRequestForRequest,
  type RateRequestInput,
} from '@/api/requests'
import {
  type RequestActionCommand,
  type RequestActionInput,
  performRequestActionForRequest,
} from '@/features/requests/request-actions-api'
import {
  type LoadActivityInput,
  loadRequestActivityForRequest,
  updateRequestDescriptionForRequest,
  type UpdateDescriptionInput,
} from '@/features/requests/request-discussion-api'
import {
  RequestDetailPage,
  RequestUnavailablePage,
} from '@/features/requests/request-detail-page'
import { RequestDetailPagePending } from '@/features/requests/request-page-pending'
import {
  loadOptionalSelectedRequestResource,
  requestParamsForRoute,
} from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import type { RequestChangesSearch } from '@/features/requests/request-changes-workbench'
import { parseRouteFileSearch } from '@/lib/route-file'
import { createFileRoute, Outlet, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback, useMemo } from 'react'

const loadRequestPage = createServerFn({ method: 'GET' })
  .validator((data: ReturnType<typeof requestParamsForRoute>) => data)
  .handler(async ({ data }) => {
    const requestParams = {
      owner: data.owner,
      repo: data.repo,
      request_id: data.request_id,
    }
    const [detail, account, ratings] = await Promise.all([
      loadOptionalRequestForRequest(requestParams),
      loadOptionalAccountSession(),
      loadOptionalSelectedRequestResource(() => loadRequestRatingsForRequest(requestParams)),
    ])
    return {
      account,
      detail,
      ratings,
    }
  })

const loadActivity = createServerFn({ method: 'GET' })
  .validator((data: LoadActivityInput) => data)
  .handler(({ data }) => loadRequestActivityForRequest(data))

const updateDescription = createServerFn({ method: 'POST' })
  .validator((data: UpdateDescriptionInput) => data)
  .handler(({ data }) => updateRequestDescriptionForRequest(data))

const runRequestAction = createServerFn({ method: 'POST' })
  .validator((data: RequestActionInput) => data)
  .handler(({ data }) => performRequestActionForRequest(data))

const rateRequest = createServerFn({ method: 'POST' })
  .validator((data: RateRequestInput) => data)
  .handler(({ data }) => rateRequestForRequest(data))

export const Route = createFileRoute('/$owner/$repo/requests/$requestId')({
  validateSearch: parseRequestDetailSearch,
  loader: ({ params }) => loadRequestPage({ data: requestParamsForRoute(params) }),
  pendingComponent: RequestDetailPagePending,
  component: RequestRoute,
})

function RequestRoute() {
  const params = Route.useParams()
  const page = Route.useLoaderData()
  const live = useRepoLayout()
  const router = useRouter()
  const navigate = Route.useNavigate()
  const repoParams = useMemo(
    () => ({ owner: params.owner, repo: params.repo }),
    [params.owner, params.repo],
  )
  const requestParams = useMemo(
    () => requestParamsForRoute({
      owner: params.owner,
      repo: params.repo,
      requestId: params.requestId,
    }),
    [params.owner, params.repo, params.requestId],
  )
  const performAction = useCallback(async (command: RequestActionCommand) => {
    const result = await runRequestAction({ data: { ...requestParams, ...command } })
    try {
      if (result.deleted) {
        await navigate({ params: repoParams, to: '/$owner/$repo/requests' })
      } else {
        await router.invalidate()
      }
      return result
    } catch {
      return {
        ...result,
        synchronizationError: 'The update completed, but the latest request state could not be reloaded. Refresh this page.',
      }
    }
  }, [navigate, repoParams, requestParams, router])
  const rateParticipant = useCallback(async (input: RateRequestInput) => {
    const rating = await rateRequest({ data: input })
    await router.invalidate()
    return rating
  }, [router])

  if (!page.detail || !page.ratings) {
    return <RequestUnavailablePage params={repoParams} />
  }

  return (
    <RequestDetailPage
      detail={page.detail}
      live={live}
      loadActivity={() => loadActivity({ data: requestParams })}
      params={repoParams}
      performAction={performAction}
      ratings={page.ratings}
      rateRequest={rateParticipant}
      updateDescription={(data) => updateDescription({ data })}
    >
      <Outlet />
    </RequestDetailPage>
  )
}

export type RequestDetailSearch = RequestChangesSearch & {
  discussion?: string
}

function parseRequestDetailSearch(
  search: Record<string, unknown>,
): RequestDetailSearch {
  return {
    commit: searchText(search.commit),
    discussion: searchText(search.discussion),
    path: searchPath(search.path),
    revision: searchText(search.revision),
  }
}

function searchPath(value: unknown) {
  const path = parseRouteFileSearch(value)
  return path ? `/${path}` : undefined
}

function searchText(value: unknown) {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

async function loadOptionalRequestForRequest(data: ReturnType<typeof requestParamsForRoute>) {
  try {
    return await loadRequestForRequest(data)
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) return null
    throw error
  }
}

async function loadOptionalAccountSession() {
  try {
    return await createApiClient().get<AccountSession>(
      buildApiPath(ApiRouteTemplates.accountSession),
      { auth: 'optional' },
    )
  } catch (error) {
    if (error instanceof HttpError && error.status === 401) return null
    throw error
  }
}
