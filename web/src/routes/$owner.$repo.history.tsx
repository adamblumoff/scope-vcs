import type { ProjectionPreviewAudience } from '@/api/types'
import { HistoryError } from '@/features/history/history-error'
import { HistoryPagePending } from '@/features/history/history-page-pending'
import { HistoryPage } from '@/features/history/history-page'
import { parseRouteFileSearch } from '@/lib/route-file'
import { loadHistoryPage } from '@/routes/-repo-history-actions'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/history')({
  validateSearch: parseHistorySearch,
  loaderDeps: ({ search }) => ({ audience: search.audience ?? null }),
  staleTime: Infinity,
  loader: ({ deps, params }) => loadHistoryPage({
    data: { ...params, audience: deps.audience, before: null },
  }),
  errorComponent: HistoryError,
  pendingComponent: HistoryPagePending,
  component: HistoryRoute,
})

function HistoryRoute() {
  const page = Route.useLoaderData()
  return (
    <HistoryPage
      initialPage={page}
      key={`${page.audience}:${page.generation}`}
      params={Route.useParams()}
      search={Route.useSearch()}
    />
  )
}

export type HistorySearch = {
  audience?: ProjectionPreviewAudience
  entry?: string
  path?: string
}

function parseHistorySearch(search: Record<string, unknown>): HistorySearch {
  return {
    audience: searchHistoryAudience(search.audience),
    entry: searchHistoryEntryId(search.entry),
    path: searchHistoryPath(search.path),
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

function searchHistoryEntryId(value: unknown) {
  if (typeof value === 'string') {
    const entryId = value.trim()
    return entryId ? entryId : undefined
  }

  if (typeof value === 'number' && Number.isFinite(value)) {
    return String(value)
  }

  return undefined
}
