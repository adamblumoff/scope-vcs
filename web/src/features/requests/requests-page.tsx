import type { RequestQueueSection } from '@/api/request-queue-input'
import type {
  RepoLiveState,
  RepoParams,
  RequestList,
  RequestListItem,
} from '@/api/types'
import { RepoShell } from '@/components/repo-shell'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { Link } from '@tanstack/react-router'
import {
  ArrowRight,
  CheckCircle2,
  Coins,
  GitPullRequest,
  Search,
  UserRound,
} from 'lucide-react'
import { type FormEvent, useReducer } from 'react'
import {
  createRequestQueueViewState,
  requestQueueViewReducer,
  requestCountLabel,
  REQUEST_QUEUE_SECTION_ORDER,
  type RequestQueuePages,
} from './request-list-model'
import {
  formatUnixDate,
  requestAudienceLabel,
  requestAuthorRoleLabel,
  requestCompletionMergeLabel,
  requestMergeabilityLabel,
  requestStatusLabel,
  requestStatusTone,
} from './request-labels'

const SECTION_DETAILS = {
  your_work: {
    description: 'Requests you authored or joined, including private drafts.',
    empty: 'No requests involve you in this repository.',
    icon: UserRound,
    title: 'Your work',
  },
  ready: {
    description: 'Highest stake first, then longest waiting.',
    empty: 'No requests are ready for review.',
    icon: GitPullRequest,
    title: 'Ready for review',
  },
  completed: {
    description: 'Public history and private results visible to maintainers.',
    empty: 'No completed requests are visible.',
    icon: CheckCircle2,
    title: 'Completed',
  },
} as const

export function RequestsPage({
  initialPages,
  live,
  loadPage,
  params,
}: {
  initialPages: RequestQueuePages
  live: RepoLiveState
  loadPage: (
    section: RequestQueueSection,
    cursor: string | null,
    search: string | null,
  ) => Promise<RequestList>
  params: RepoParams
}) {
  const [state, dispatch] = useReducer(
    requestQueueViewReducer,
    initialPages,
    createRequestQueueViewState,
  )
  const {
    loadingSection,
    pages,
    searchDraft,
    searchError,
    searching,
    searchQuery,
    sectionErrors,
  } = state

  async function loadMore(section: RequestQueueSection) {
    const cursor = pages[section].next_cursor
    if (!cursor || loadingSection || searching) return

    dispatch({ type: 'load_started', section })
    try {
      const page = await loadPage(
        section,
        cursor,
        section === 'your_work' ? null : searchQuery || null,
      )
      dispatch({ type: 'load_succeeded', section, page })
    } catch (error) {
      dispatch({
        type: 'load_failed',
        section,
        error: errorMessage(
          error,
          `Could not load more ${SECTION_DETAILS[section].title.toLowerCase()} requests.`,
        ),
      })
    }
  }

  async function searchQueue(query: string) {
    if (searching || loadingSection) return
    const normalizedQuery = query.trim()
    if (normalizedQuery === searchQuery) return

    dispatch({ type: 'search_started' })
    try {
      const [ready, completed] = await Promise.all([
        loadPage('ready', null, normalizedQuery || null),
        loadPage('completed', null, normalizedQuery || null),
      ])
      dispatch({
        type: 'search_succeeded',
        query: normalizedQuery,
        ready,
        completed,
      })
    } catch (error) {
      dispatch({
        type: 'search_failed',
        error: errorMessage(error, 'Could not search requests.'),
      })
    }
  }

  function submitSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    void searchQueue(searchDraft)
  }

  function clearSearch() {
    dispatch({ type: 'search_draft_changed', value: '' })
    void searchQueue('')
  }

  return (
    <RepoShell params={params}>
      <WorkbenchHeader
        count={`${live.repo.ready_for_review_count} ready for review`}
        description="Review repository work by attention, then keep completed decisions easy to find."
        eyebrow="Review"
        title="Requests"
      />
      <div className="px-4 pb-12 sm:px-6 lg:px-8">
        <QueueSearch
          busy={Boolean(loadingSection) || searching}
          error={searchError}
          onChange={(value) => dispatch({ type: 'search_draft_changed', value })}
          onClear={clearSearch}
          onSubmit={submitSearch}
          query={searchDraft}
          searching={searching}
          searchQuery={searchQuery}
        />
        <div aria-busy={searching} className="divide-y divide-border">
          {REQUEST_QUEUE_SECTION_ORDER.map((section) => (
            <QueueSection
              busy={Boolean(loadingSection) || searching}
              error={sectionErrors[section]}
              key={section}
              loading={loadingSection === section}
              onLoadMore={() => void loadMore(section)}
              page={pages[section]}
              params={params}
              searchQuery={section === 'your_work' ? '' : searchQuery}
              section={section}
            />
          ))}
        </div>
      </div>
    </RepoShell>
  )
}

function QueueSearch({
  busy,
  error,
  onChange,
  onClear,
  onSubmit,
  query,
  searching,
  searchQuery,
}: {
  busy: boolean
  error: string | null
  onChange: (value: string) => void
  onClear: () => void
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
  query: string
  searching: boolean
  searchQuery: string
}) {
  return (
    <form
      className="flex flex-col gap-2 border-b border-border py-5 sm:flex-row sm:items-center"
      onSubmit={onSubmit}
      role="search"
    >
      <label className="relative block min-w-0 flex-1 sm:max-w-lg">
        <span className="sr-only">Search ready and completed requests</span>
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
        />
        <input
          className="h-10 w-full rounded-md border border-input bg-background pl-9 pr-3 text-sm text-foreground placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          disabled={busy}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Search ready and completed requests"
          type="search"
          value={query}
        />
      </label>
      <div className="flex items-center gap-2">
        <Button disabled={busy} size="sm" type="submit" variant="secondary">
          {searching ? 'Searching…' : 'Search'}
        </Button>
        {searchQuery ? (
          <Button
            disabled={busy}
            onClick={onClear}
            size="sm"
            type="button"
            variant="ghost"
          >
            Clear
          </Button>
        ) : null}
      </div>
      {error ? (
        <p className="text-sm text-destructive sm:ml-2" role="alert">
          {error}
        </p>
      ) : null}
      {searching ? <output className="sr-only">Searching requests…</output> : null}
    </form>
  )
}

function QueueSection({
  busy,
  error,
  loading,
  onLoadMore,
  page,
  params,
  searchQuery,
  section,
}: {
  busy: boolean
  error?: string
  loading: boolean
  onLoadMore: () => void
  page: RequestList
  params: RepoParams
  searchQuery: string
  section: RequestQueueSection
}) {
  const details = SECTION_DETAILS[section]
  const Icon = details.icon
  const headingId = `request-queue-${section}`
  const emptyMessage = searchQuery
    ? `No ${details.title.toLowerCase()} requests match “${searchQuery}”.`
    : details.empty

  return (
    <section aria-labelledby={headingId} className="py-7 sm:py-8">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Icon aria-hidden="true" className="size-4 text-muted-foreground" />
            <h2 className="text-base font-semibold tracking-[-0.012em]" id={headingId}>
              {details.title}
            </h2>
          </div>
          <p className="mt-1 text-sm leading-5 text-muted-foreground">
            {details.description}
          </p>
        </div>
        <span className="text-xs tabular-nums text-muted-foreground">
          {requestCountLabel(page.requests.length, Boolean(page.next_cursor))}
        </span>
      </div>

      {page.requests.length ? (
        <div className="mt-4 divide-y divide-border border-y border-border">
          {page.requests.map((request) => (
            <RequestQueueRow
              key={request.id}
              params={params}
              request={request}
              section={section}
            />
          ))}
        </div>
      ) : (
        <p className="mt-4 border-y border-border py-6 text-sm text-muted-foreground">
          {emptyMessage}
        </p>
      )}

      {page.next_cursor ? (
        <div className="pt-4">
          <Button
            disabled={busy}
            onClick={onLoadMore}
            size="sm"
            type="button"
            variant="secondary"
          >
            {loading ? 'Loading…' : `Load more ${details.title.toLowerCase()}`}
          </Button>
          {loading ? (
            <output className="sr-only">
              Loading more {details.title.toLowerCase()} requests…
            </output>
          ) : null}
        </div>
      ) : null}
      {error ? (
        <p className="mt-2 text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  )
}

function RequestQueueRow({
  params,
  request,
  section,
}: {
  params: RepoParams
  request: RequestListItem
  section: RequestQueueSection
}) {
  return (
    <Link
      className="group grid min-w-0 gap-3 py-4 outline-none transition-colors [contain-intrinsic-size:auto_76px] [content-visibility:auto] hover:bg-muted/45 focus-visible:bg-muted/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:px-3"
      params={{ ...params, requestId: request.id }}
      to="/repos/$owner/$repo/requests/$requestId"
    >
      <div className="min-w-0">
        <h3 className="break-words text-sm font-semibold leading-6 tracking-[-0.008em] group-hover:underline">
          {request.title}
        </h3>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs leading-5 text-muted-foreground">
          {request.title !== request.name ? (
            <>
              <span className="font-mono">{request.name}</span>
              <MetadataSeparator />
            </>
          ) : null}
          <span>{requestAudienceLabel(request)}</span>
          <MetadataSeparator />
          <span>{requestAuthorRoleLabel(request)}</span>
          <MetadataSeparator />
          <span className="font-mono">{request.id}</span>
          <MetadataSeparator />
          <QueueDate request={request} section={section} />
          {section === 'your_work' ? (
            <>
              <MetadataSeparator />
              <span>{requestStatusLabel(request)}</span>
            </>
          ) : null}
        </div>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-2 sm:justify-end">
        {request.held_at_unix !== null ? <Badge variant="warning">On hold</Badge> : null}
        {request.current_stake_credits > 0 ? (
          <span className="inline-flex items-center gap-1 text-xs font-medium tabular-nums text-foreground">
            <Coins aria-hidden="true" className="size-3.5 text-muted-foreground" />
            {request.current_stake_credits} staked
          </span>
        ) : null}
        {section === 'completed' ? (
          <>
            <span className="text-xs text-muted-foreground">
              {requestCompletionMergeLabel(request)}
            </span>
            <Badge variant={requestStatusTone(request)}>
              {requestStatusLabel(request)}
            </Badge>
          </>
        ) : section === 'ready' ? (
          <span className="text-xs text-muted-foreground">
            {requestMergeabilityLabel(request)}
          </span>
        ) : null}
        <ArrowRight aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
      </div>
    </Link>
  )
}

function QueueDate({
  request,
  section,
}: {
  request: RequestListItem
  section: RequestQueueSection
}) {
  if (section === 'ready' && request.ready_at_unix !== null) {
    return <span className="tabular-nums">Ready {formatUnixDate(request.ready_at_unix)}</span>
  }
  return <span className="tabular-nums">Updated {formatUnixDate(request.updated_at_unix)}</span>
}

function MetadataSeparator() {
  return <span aria-hidden="true">·</span>
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback
}
