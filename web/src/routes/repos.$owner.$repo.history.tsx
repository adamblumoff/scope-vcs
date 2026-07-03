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
import {
  loadCommitDetail,
  loadCommitHistory,
} from '@/routes/-repo-history-actions'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/repos/$owner/$repo/history')({
  validateSearch: parseHistorySearch,
  loaderDeps: ({ search }) => search,
  loader: async ({ deps: search, params }) => {
    const [privateHistory, publicHistory] = await Promise.all([
      loadOptionalPrivateHistory(params),
      loadPublicHistory(params),
    ])

    if (!privateHistory && !publicHistory.history) {
      throw publicHistory.error
    }

    const histories = {
      private: privateHistory,
      public: publicHistory.history,
    } satisfies CommitHistories
    const initialAudience = initialHistoryAudience(histories, search)
    const initialCommitId =
      selectedCommitId(histories[initialAudience], search.commit) ??
      latestCommitId(histories[initialAudience])
    const initialCommit = initialCommitId
      ? await loadCommitDetail({
          data: {
            ...params,
            audience: initialAudience,
            commit: initialCommitId,
          },
        })
      : null

    return {
      histories,
      initialAudience,
      initialCommit,
    }
  },
  errorComponent: HistoryError,
  component: HistoryRoute,
})

function HistoryRoute() {
  const params = Route.useParams()
  const { histories, initialAudience, initialCommit } = Route.useLoaderData()

  return (
    <HistoryPage
      histories={histories}
      initialAudience={initialAudience}
      initialCommit={initialCommit}
      params={params}
    />
  )
}

type HistorySearch = {
  audience?: ProjectionPreviewAudience
  commit?: string
}

function parseHistorySearch(search: Record<string, unknown>): HistorySearch {
  return {
    audience: searchHistoryAudience(search.audience),
    commit: searchCommitId(search.commit),
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

async function loadOptionalPrivateHistory(params: RepoParams) {
  try {
    return await loadCommitHistory({
      data: { ...params, audience: 'private' },
    })
  } catch (error) {
    if (isForbiddenOrNotFound(error)) {
      return null
    }
    throw error
  }
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

function initialHistoryAudience(
  histories: CommitHistories,
  search: HistorySearch,
): ProjectionPreviewAudience {
  if (search.audience && histories[search.audience]) {
    return search.audience
  }
  return histories.private ? 'private' : 'public'
}

function selectedCommitId(history: CommitHistory | null, commitId?: string) {
  if (!commitId) {
    return null
  }
  return history?.commits.some((commit) => commit.projected_id === commitId)
    ? commitId
    : null
}

function latestCommitId(history: CommitHistory | null) {
  return history?.commits.at(-1)?.projected_id ?? null
}

function isForbiddenOrNotFound(error: unknown) {
  return (
    typeof error === 'object' &&
    error !== null &&
    'status' in error &&
    [403, 404].includes(Number((error as { status: unknown }).status))
  )
}
