import { createApiClient, HttpError } from '@/api/client'
import type { AccountSession } from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import {
  loadRequestForRequest,
} from '@/api/repos'
import {
  type LoadRequestChangeBlockFilesInput,
  loadRequestChangeBlockFilesForRequest,
} from '@/api/requests'
import {
  createRequestDiscussionForRequest,
  createRequestDiscussionReplyForRequest,
  type CreateDiscussionInput,
  type CreateReplyInput,
  type LoadActivityInput,
  loadRequestActivityForRequest,
  loadRequestDiscussionRepliesForRequest,
  loadRequestDiscussionChangesForRequest,
  loadRequestDiscussionsForRequest,
  type LoadDiscussionsInput,
  type LoadRepliesInput,
  markRequestDiscussionReadForRequest,
  type MarkDiscussionReadInput,
  reopenAndReplyToRequestDiscussionForRequest,
  reopenRequestDiscussionForRequest,
  type RequestDiscussionActionInput,
  resolveRequestDiscussionForRequest,
  updateRequestDescriptionForRequest,
  type UpdateDescriptionInput,
} from '@/features/requests/request-discussion-api'
import {
  RequestDetailPage,
  RequestUnavailablePage,
} from '@/features/requests/request-detail-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRequestPage = createServerFn({ method: 'GET' })
  .validator((data: ReturnType<typeof requestParamsForRoute>) => data)
  .handler(async ({ data }) => {
    const [detail, account, discussionPage] = await Promise.all([
      loadOptionalRequestForRequest(data),
      loadOptionalAccountSession(),
      loadOptionalSelectedRequestResource(() =>
        loadRequestDiscussionsForRequest(data),
      ),
    ])
    return { account, detail, discussionPage }
  })

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator((data: LoadDiscussionsInput) => data)
  .handler(({ data }) => loadRequestDiscussionsForRequest(data))

const loadActivity = createServerFn({ method: 'GET' })
  .validator((data: LoadActivityInput) => data)
  .handler(({ data }) => loadRequestActivityForRequest(data))

const loadReplies = createServerFn({ method: 'GET' })
  .validator((data: LoadRepliesInput) => data)
  .handler(({ data }) => loadRequestDiscussionRepliesForRequest(data))

const loadDiscussionChanges = createServerFn({ method: 'GET' })
  .validator((data: ReturnType<typeof requestParamsForRoute> & { after: number }) => data)
  .handler(({ data }) => loadRequestDiscussionChangesForRequest(data))

const loadChangeBlockFiles = createServerFn({ method: 'GET' })
  .validator((data: LoadRequestChangeBlockFilesInput) => data)
  .handler(({ data }) => loadRequestChangeBlockFilesForRequest(data))

const createDiscussion = createServerFn({ method: 'POST' })
  .validator((data: CreateDiscussionInput) => data)
  .handler(({ data }) => createRequestDiscussionForRequest(data))

const createReply = createServerFn({ method: 'POST' })
  .validator((data: CreateReplyInput) => data)
  .handler(({ data }) => createRequestDiscussionReplyForRequest(data))

const resolveDiscussion = createServerFn({ method: 'POST' })
  .validator((data: RequestDiscussionActionInput) => data)
  .handler(({ data }) => resolveRequestDiscussionForRequest(data))

const reopenDiscussion = createServerFn({ method: 'POST' })
  .validator((data: RequestDiscussionActionInput) => data)
  .handler(({ data }) => reopenRequestDiscussionForRequest(data))

const reopenAndReply = createServerFn({ method: 'POST' })
  .validator((data: CreateReplyInput) => data)
  .handler(({ data }) => reopenAndReplyToRequestDiscussionForRequest(data))

const markDiscussionRead = createServerFn({ method: 'POST' })
  .validator((data: MarkDiscussionReadInput) => data)
  .handler(({ data }) => markRequestDiscussionReadForRequest(data))

const updateDescription = createServerFn({ method: 'POST' })
  .validator((data: UpdateDescriptionInput) => data)
  .handler(({ data }) => updateRequestDescriptionForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/requests/$requestId')({
  loader: ({ params }) => loadRequestPage({ data: requestParamsForRoute(params) }),
  component: RequestRoute,
})

function RequestRoute() {
  const params = Route.useParams()
  const page = Route.useLoaderData()
  const live = useRepoLayout()
  const repoParams = { owner: params.owner, repo: params.repo }

  if (!page.detail || !page.discussionPage) {
    return <RequestUnavailablePage params={repoParams} />
  }

  return (
    <RequestDetailPage
      actor={page.account?.user ?? null}
      createDiscussion={(data) => createDiscussion({ data })}
      createReply={(data) => createReply({ data })}
      detail={page.detail}
      discussionPage={page.discussionPage}
      live={live}
      loadActivity={() => loadActivity({ data: requestParamsForRoute(params) })}
      loadChangeBlockFiles={(data) => loadChangeBlockFiles({ data })}
      loadDiscussions={(data) => loadDiscussions({ data })}
      loadDiscussionChanges={(data) => loadDiscussionChanges({ data })}
      loadReplies={(data) => loadReplies({ data })}
      markDiscussionRead={(data) => markDiscussionRead({ data })}
      params={repoParams}
      reopenAndReply={(data) => reopenAndReply({ data })}
      reopenDiscussion={(data) => reopenDiscussion({ data })}
      resolveDiscussion={(data) => resolveDiscussion({ data })}
      updateDescription={(data) => updateDescription({ data })}
    />
  )
}

function requestParamsForRoute(params: { owner: string; repo: string; requestId: string }) {
  return { owner: params.owner, repo: params.repo, request_id: params.requestId }
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

async function loadOptionalSelectedRequestResource<T>(load: () => Promise<T>) {
  try {
    return await load()
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) return null
    throw error
  }
}
