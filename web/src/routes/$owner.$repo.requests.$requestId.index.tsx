import {
  createRequestDiscussionForRequest,
  createRequestDiscussionReplyForRequest,
  type CreateDiscussionInput,
  type CreateReplyInput,
  type LoadDiscussionsInput,
  type LoadRepliesInput,
  type MarkDiscussionReadInput,
  type RequestDiscussionActionInput,
  loadRequestDiscussionChangesForRequest,
  loadRequestDiscussionRepliesForRequest,
  loadRequestDiscussionsForRequest,
  markRequestDiscussionReadForRequest,
  reopenAndReplyToRequestDiscussionForRequest,
  resolveRequestDiscussionForRequest,
} from '@/features/requests/request-discussion-api'
import { includeFocusedDiscussion } from '@/features/requests/request-discussion-model'
import { RequestDiscussionView } from '@/features/requests/request-discussion-view'
import {
  loadOptionalSelectedRequestResource,
  requestParamsForRoute,
} from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute, getRouteApi } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { MessageSquare } from 'lucide-react'

const requestRoute = getRouteApi('/$owner/$repo/requests/$requestId')

const loadDiscussionPage = createServerFn({ method: 'GET' })
  .validator((data: ReturnType<typeof requestParamsForRoute> & { discussion_id?: string }) => data)
  .handler(async ({ data }) => {
    const requestParams = {
      owner: data.owner,
      repo: data.repo,
      request_id: data.request_id,
    }
    const [discussionPage, focusedDiscussionPage] = await Promise.all([
      loadOptionalSelectedRequestResource(() => loadRequestDiscussionsForRequest(requestParams)),
      data.discussion_id
        ? loadOptionalSelectedRequestResource(() => loadRequestDiscussionsForRequest(data))
        : Promise.resolve(null),
    ])
    return includeFocusedDiscussion(discussionPage, focusedDiscussionPage)
  })

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator((data: LoadDiscussionsInput) => data)
  .handler(({ data }) => loadRequestDiscussionsForRequest(data))

const loadReplies = createServerFn({ method: 'GET' })
  .validator((data: LoadRepliesInput) => data)
  .handler(({ data }) => loadRequestDiscussionRepliesForRequest(data))

const loadDiscussionChanges = createServerFn({ method: 'GET' })
  .validator((data: ReturnType<typeof requestParamsForRoute> & { after: number }) => data)
  .handler(({ data }) => loadRequestDiscussionChangesForRequest(data))

const createDiscussion = createServerFn({ method: 'POST' })
  .validator((data: CreateDiscussionInput) => data)
  .handler(({ data }) => createRequestDiscussionForRequest(data))

const createReply = createServerFn({ method: 'POST' })
  .validator((data: CreateReplyInput) => data)
  .handler(({ data }) => createRequestDiscussionReplyForRequest(data))

const resolveDiscussion = createServerFn({ method: 'POST' })
  .validator((data: RequestDiscussionActionInput) => data)
  .handler(({ data }) => resolveRequestDiscussionForRequest(data))

const reopenAndReply = createServerFn({ method: 'POST' })
  .validator((data: CreateReplyInput) => data)
  .handler(({ data }) => reopenAndReplyToRequestDiscussionForRequest(data))

const markDiscussionRead = createServerFn({ method: 'POST' })
  .validator((data: MarkDiscussionReadInput) => data)
  .handler(({ data }) => markRequestDiscussionReadForRequest(data))

export const Route = createFileRoute('/$owner/$repo/requests/$requestId/')({
  loaderDeps: ({ search }) => ({ discussion: search.discussion }),
  loader: ({ deps, params }) => loadDiscussionPage({
    data: {
      ...requestParamsForRoute(params),
      discussion_id: deps.discussion,
    },
  }),
  component: RequestDiscussionRoute,
})

function RequestDiscussionRoute() {
  const page = requestRoute.useLoaderData()
  const initialPage = Route.useLoaderData()
  const params = Route.useParams()
  const search = Route.useSearch()
  const live = useRepoLayout()

  if (!page.detail) return null
  if (!initialPage) {
    return (
      <section className="px-5 py-14 text-center lg:px-7">
        <MessageSquare className="mx-auto size-5 text-muted-foreground" />
        <h2 className="mt-3 text-sm font-semibold">Discussion is unavailable</h2>
        <p className="mx-auto mt-1 max-w-md text-sm leading-6 text-muted-foreground">
          The request is still available. Reload the page to try loading its discussion again.
        </p>
      </section>
    )
  }

  return (
    <RequestDiscussionView
      account={page.account}
      createDiscussion={(data) => createDiscussion({ data })}
      createReply={(data) => createReply({ data })}
      detail={page.detail}
      focusedDiscussionId={search.discussion}
      initialPage={initialPage}
      live={live}
      loadDiscussions={(data) => loadDiscussions({ data })}
      loadDiscussionChanges={(data) => loadDiscussionChanges({ data })}
      loadReplies={(data) => loadReplies({ data })}
      markDiscussionRead={(data) => markDiscussionRead({ data })}
      params={{ owner: params.owner, repo: params.repo }}
      reopenAndReply={(data) => reopenAndReply({ data })}
      resolveDiscussion={(data) => resolveDiscussion({ data })}
    />
  )
}
