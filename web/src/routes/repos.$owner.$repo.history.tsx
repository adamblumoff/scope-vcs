import type {
  CommitHistory,
  ProjectionPreviewAudience,
  RepoParams,
} from '@/api/types'
import {
  HistoryError,
  HistoryPage,
  type CommitHistories,
} from '@/features/history/history-page'
import { parseRouteFileSearch } from '@/lib/route-file'
import {
  loadCommitHistory,
  loadOptionalPrivateCommitHistory,
} from '@/routes/-repo-history-actions'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/repos/$owner/$repo/history')({
  validateSearch: parseHistorySearch,
  staleTime: Infinity,
  loader: async ({ params }) => {
    const [privateHistory, publicHistory] = await Promise.all([
      loadOptionalPrivateHistory(params),
      loadPublicHistory(params),
    ])

    if (!privateHistory && !publicHistory.history) {
      throw publicHistory.error
    }

    return {
      private: privateHistory,
      public: publicHistory.history,
    } satisfies CommitHistories
  },
  errorComponent: HistoryError,
  component: HistoryRoute,
})

function HistoryRoute() {
  return (
    <HistoryPage
      histories={Route.useLoaderData()}
      params={Route.useParams()}
      search={Route.useSearch()}
    />
  )
}

export type HistorySearch = {
  audience?: ProjectionPreviewAudience
  commit?: string
  path?: string
  request?: string
  revision?: string
}

function parseHistorySearch(search: Record<string, unknown>): HistorySearch {
  return {
    audience: searchHistoryAudience(search.audience),
    commit: searchCommitId(search.commit),
    path: searchHistoryPath(search.path),
    request: searchText(search.request),
    revision: searchText(search.revision),
  }
}

function searchHistoryAudience(value: unknown): ProjectionPreviewAudience | undefined {
  if (value === undefined || value === null || value === '') {
    return undefined
  }
  if (value === 'private' || value === 'public') {
    return value
  }
  throw new Error(`Unsupported history audience: ${String(value)}`)
}

function searchHistoryPath(value: unknown) {
  const path = parseRouteFileSearch(value)
  return path ? `/${path}` : undefined
}

function searchCommitId(value: unknown) {
  if (typeof value === 'string') {
    const commitId = value.trim()
    return commitId ? commitId : undefined
  }

  if (typeof value === 'number' && Number.isFinite(value)) {
    return String(value)
  }

  return undefined
}

function searchText(value: unknown) {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

async function loadOptionalPrivateHistory(params: RepoParams) {
  return loadOptionalPrivateCommitHistory({
    data: { ...params, audience: 'private' },
  })
}

async function loadPublicHistory(params: RepoParams): Promise<{
  error: unknown
  history: CommitHistory | null
}> {
  try {
    return {
      error: null,
      history: await loadCommitHistory({
        data: { ...params, audience: 'public' },
      }),
    }
  } catch (error) {
    return { error, history: null }
  }
}
