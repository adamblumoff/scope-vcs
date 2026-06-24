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
  loadCommitFileDiff,
  loadCommitHistory,
} from '@/routes/-repo-history-actions'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/repos/$owner/$repo/history')({
  validateSearch: (search: Record<string, unknown>): HistorySearch => ({
    audience:
      search.audience === 'owner' || search.audience === 'public'
        ? search.audience
        : undefined,
    commit: searchCommitId(search.commit),
  }),
  loaderDeps: ({ search }) => search,
  loader: async ({ deps: search, params }) => {
    const [ownerHistory, publicHistory] = await Promise.all([
      loadOptionalOwnerHistory(params),
      loadPublicHistory(params),
    ])

    if (!ownerHistory && !publicHistory.history) {
      throw publicHistory.error
    }

    const histories = {
      owner: ownerHistory,
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
      loadCommit={(data) => loadCommitDetail({ data })}
      loadFileDiff={(data) => loadCommitFileDiff({ data })}
      params={params}
    />
  )
}

type HistorySearch = {
  audience?: ProjectionPreviewAudience
  commit?: string
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

async function loadOptionalOwnerHistory(params: RepoParams) {
  try {
    return await loadCommitHistory({
      data: { ...params, audience: 'owner' },
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
  return histories.owner ? 'owner' : 'public'
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
