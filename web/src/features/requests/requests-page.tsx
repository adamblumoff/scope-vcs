import type { RequestQueueSection } from '@/api/request-queue-input'
import type {
  RepoParams,
  RequestList,
  RequestListItem,
} from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/empty-state'
import { PageContent } from '@/components/page-header'
import { Link } from '@tanstack/react-router'
import {
  CheckCircle2,
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
  requestAuthorRoleLabel,
  requestMergeabilityLabel,
  requestStatusLabel,
  requestStatusTone,
} from './request-labels'

const SECTION_DETAILS = {
  your_work: {
    empty: 'Nothing here involves you yet.',
    icon: UserRound,
    title: 'Your work',
  },
  open: {
    empty: 'No open requests.',
    icon: GitPullRequest,
    title: 'Open',
  },
  closed: {
    empty: 'No closed requests.',
    icon: CheckCircle2,
    title: 'Closed',
  },
} as const

export function RequestsPage({
  initialPages,
  loadPage,
  params,
}: {
  initialPages: RequestQueuePages
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
  if (state.snapshot !== initialPages) {
    dispatch({ type: 'loader_snapshot_received', pages: initialPages })
  }
  const {
    generation,
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

    const operationGeneration = generation
    dispatch({
      type: 'load_started',
      generation: operationGeneration,
      section,
    })
    try {
      const page = await loadPage(
        section,
        cursor,
        section === 'your_work' ? null : searchQuery || null,
      )
      dispatch({
        type: 'load_succeeded',
        generation: operationGeneration,
        section,
        page,
      })
    } catch (error) {
      dispatch({
        type: 'load_failed',
        generation: operationGeneration,
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

    const operationGeneration = generation
    dispatch({ type: 'search_started', generation: operationGeneration })
    try {
      const [open, closed] = await Promise.all([
        loadPage('open', null, normalizedQuery || null),
        loadPage('closed', null, normalizedQuery || null),
      ])
      dispatch({
        type: 'search_succeeded',
        generation: operationGeneration,
        query: normalizedQuery,
        open,
        closed,
      })
    } catch (error) {
      dispatch({
        type: 'search_failed',
        generation: operationGeneration,
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
    <PageContent className="pb-16">
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
      <div aria-busy={searching} className="mt-10 grid gap-12">
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
    </PageContent>
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
      className="flex flex-col gap-2 sm:flex-row sm:items-center"
      onSubmit={onSubmit}
      role="search"
    >
      <label className="relative block min-w-0 flex-1 sm:max-w-lg">
        <span className="sr-only">Search open and closed requests</span>
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
        />
        <input
          className="h-10 w-full rounded-md border border-input bg-background pl-9 pr-3 text-sm text-foreground placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          disabled={busy}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Search requests"
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
    ? `Nothing matches “${searchQuery}”.`
    : details.empty

  return (
    <section aria-labelledby={headingId}>
      <div className="flex items-center gap-2">
        <Icon aria-hidden="true" className="size-4 text-muted-foreground" />
        <h2 className="text-sm font-semibold" id={headingId}>
          {details.title}
        </h2>
        <span className="text-xs tabular-nums text-muted-foreground">
          {requestCountLabel(page.requests.length, Boolean(page.next_cursor))}
        </span>
      </div>

      {page.requests.length ? (
        <div className="mt-2 divide-y divide-border">
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
        <EmptyState className="mt-3" inline title={emptyMessage} />
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
            {loading ? 'Loading…' : 'Load more'}
          </Button>
          {loading ? (
            <output className="sr-only">
              Loading more {details.title.toLowerCase()} requests…
            </output>
          ) : null}
        </div>
      ) : null}
      {error ? (
        <p className="mt-2 text-sm text-danger-strong" role="alert">
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
      className="group block min-w-0 rounded-md py-3 outline-none transition-colors [contain-intrinsic-size:auto_64px] [content-visibility:auto] hover:bg-accent/50 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      params={{ ...params, requestId: request.id }}
      title={request.id}
      to="/$owner/$repo/requests/$requestId"
    >
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-2.5 gap-y-1">
        <h3 className="break-words text-sm font-medium leading-6 group-hover:underline">
          {request.title}
        </h3>
        {request.title !== request.name ? (
          <span className="truncate font-mono text-xs text-muted-foreground">
            {request.name}
          </span>
        ) : null}
      </div>
      {/* Status stays adjacent to the title rather than justified to the far
          edge, so wide viewports do not separate a row from its state. */}
      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs leading-5 text-muted-foreground">
        {section === 'open' ? (
          <span>{requestMergeabilityLabel(request)}</span>
        ) : (
          <Badge variant={requestStatusTone(request)}>
            {requestStatusLabel(request)}
          </Badge>
        )}
        <span aria-hidden="true">·</span>
        <span>{requestAuthorRoleLabel(request)}</span>
        <span aria-hidden="true">·</span>
        <QueueDate request={request} section={section} />
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
  if (section === 'open' && request.submitted_at_unix !== null) {
    return <span className="tabular-nums">Submitted {formatUnixDate(request.submitted_at_unix)}</span>
  }
  return <span className="tabular-nums">Updated {formatUnixDate(request.updated_at_unix)}</span>
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback
}
