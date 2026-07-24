import { loadRequestQueueForRequest } from '@/api/repos'
import {
  parseLoadRequestQueueInput,
  type RequestQueueSection,
} from '@/api/request-queue-input'
import { RequestsPage } from '@/features/requests/requests-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const loadRequestQueuePage = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestQueueInput)
  .handler(async ({ data }) => loadRequestQueueForRequest(data))

function createRefreshKey() {
  return Array.from(
    globalThis.crypto.getRandomValues(new Uint32Array(4)),
    (value) => value.toString(36),
  ).join('-')
}

export const Route = createFileRoute('/repos/$owner/$repo/requests/')({
  loader: async ({ params }) => {
    const [yourWork, ready, completed] = await Promise.all([
      loadRequestQueuePage({ data: { ...params, section: 'your_work' } }),
      loadRequestQueuePage({ data: { ...params, section: 'ready' } }),
      loadRequestQueuePage({ data: { ...params, section: 'completed' } }),
    ])
    return {
      initialPages: { completed, ready, your_work: yourWork },
      refreshKey: createRefreshKey(),
    }
  },
  component: RequestsRoute,
})

function RequestsRoute() {
  const params = Route.useParams()
  const { owner, repo } = params
  const live = useRepoLayout()
  const { initialPages, refreshKey } = Route.useLoaderData()
  const loadPage = useCallback(
    (
      section: RequestQueueSection,
      cursor: string | null,
      search: string | null,
    ) =>
      loadRequestQueuePage({
        data: { cursor, owner, repo, search, section },
      }),
    [owner, repo],
  )

  return (
    <RequestsPage
      initialPages={initialPages}
      key={`${owner}/${repo}:${refreshKey}`}
      live={live}
      loadPage={loadPage}
      params={params}
    />
  )
}
