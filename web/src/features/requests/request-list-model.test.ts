import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestList, RequestListItem } from '@/api/types'
import {
  appendQueuePage,
  appendRequestPage,
  createRequestQueueViewState,
  requestQueueViewReducer,
  requestCountLabel,
  type RequestQueuePages,
  type RequestQueueViewAction,
} from './request-list-model'

test('appendRequestPage preserves order and ignores repeated request ids', () => {
  const first = request('req_1')
  const repeated = request('req_1')
  const second = request('req_2')

  assert.deepEqual(appendRequestPage([first], [repeated, second, second]), [
    first,
    second,
  ])
})

test('requestCountLabel marks partial counts until the final page', () => {
  assert.equal(requestCountLabel(50, true), '50+ requests')
  assert.equal(requestCountLabel(51, false), '51 requests')
  assert.equal(requestCountLabel(1, false), '1 request')
})

test('appendQueuePage advances a page without duplicating rows', () => {
  const first = request('req_1')
  const second = request('req_2')
  const current: RequestList = {
    requests: [first],
    next_cursor: 'open:page-2',
  }
  const incoming: RequestList = {
    requests: [first, second],
    next_cursor: 'open:page-3',
  }

  assert.deepEqual(appendQueuePage(current, incoming), {
    requests: [first, second],
    next_cursor: 'open:page-3',
  })
  assert.equal(current.next_cursor, 'open:page-2')
})

test('request queue reducer exposes loading success and error states', () => {
  const initial = createRequestQueueViewState(queuePages())
  const loading = requestQueueViewReducer(initial, {
    type: 'load_started',
    generation: initial.generation,
    section: 'open',
  })
  assert.equal(loading.loadingSection, 'open')

  const loaded = requestQueueViewReducer(loading, {
    type: 'load_succeeded',
    generation: loading.generation,
    section: 'open',
    page: page(['open-1', 'open-2'], null),
  })
  assert.equal(loaded.loadingSection, null)
  assert.deepEqual(
    loaded.pages.open.requests.map(({ id }) => id),
    ['open-1', 'open-2'],
  )

  const failed = requestQueueViewReducer(
    requestQueueViewReducer(loaded, {
      type: 'load_started',
      generation: loaded.generation,
      section: 'closed',
    }),
    {
      type: 'load_failed',
      generation: loaded.generation,
      section: 'closed',
      error: 'Could not load closed requests.',
    },
  )
  assert.equal(failed.loadingSection, null)
  assert.equal(
    failed.sectionErrors.closed,
    'Could not load closed requests.',
  )
})

test('request queue reducer replaces searched sections and preserves Your work', () => {
  const initial = {
    ...createRequestQueueViewState(queuePages()),
    searchError: 'Previous failure',
    sectionErrors: {
      open: 'Open failure',
      closed: 'Closed failure',
    },
  }
  const searching = requestQueueViewReducer(initial, {
    type: 'search_started',
    generation: initial.generation,
  })
  assert.equal(searching.searching, true)
  assert.equal(searching.searchError, null)

  const searched = requestQueueViewReducer(searching, {
    type: 'search_succeeded',
    generation: searching.generation,
    query: 'needle',
    open: page(['open-search'], null),
    closed: page(['closed-search'], null),
  })
  assert.equal(searched.searching, false)
  assert.equal(searched.searchQuery, 'needle')
  assert.deepEqual(
    searched.pages.your_work.requests.map(({ id }) => id),
    ['work-1'],
  )
  assert.deepEqual(
    searched.pages.open.requests.map(({ id }) => id),
    ['open-search'],
  )
  assert.deepEqual(
    searched.pages.closed.requests.map(({ id }) => id),
    ['closed-search'],
  )
  assert.equal(searched.sectionErrors.open, undefined)
  assert.equal(searched.sectionErrors.closed, undefined)

  const failed = requestQueueViewReducer(
    requestQueueViewReducer(searched, {
      type: 'search_started',
      generation: searched.generation,
    }),
    {
      type: 'search_failed',
      generation: searched.generation,
      error: 'Search unavailable.',
    },
  )
  assert.equal(failed.searching, false)
  assert.equal(failed.searchError, 'Search unavailable.')
})

test('request queue reducer replaces stale state from an authoritative snapshot', () => {
  const initial = {
    ...createRequestQueueViewState(queuePages()),
    loadingSection: 'open' as const,
    searchDraft: 'old query',
    searchQuery: 'old query',
    searching: true,
  }
  const replacement = queuePages({
    open: page(['open-new'], null),
  })

  const refreshed = requestQueueViewReducer(initial, {
    type: 'loader_snapshot_received',
    pages: replacement,
  })

  assert.equal(refreshed.generation, initial.generation + 1)
  assert.equal(refreshed.loadingSection, null)
  assert.equal(refreshed.searchDraft, '')
  assert.equal(refreshed.searchQuery, '')
  assert.equal(refreshed.searching, false)
  assert.deepEqual(refreshed.pages, replacement)

  const staleActions: RequestQueueViewAction[] = [
    {
      type: 'load_succeeded',
      generation: initial.generation,
      section: 'open',
      page: page(['open-stale'], null),
    },
    {
      type: 'load_failed',
      generation: initial.generation,
      section: 'open',
      error: 'Stale load failure',
    },
    {
      type: 'search_succeeded',
      generation: initial.generation,
      query: 'stale',
      open: page(['open-stale'], null),
      closed: page(['closed-stale'], null),
    },
    {
      type: 'search_failed',
      generation: initial.generation,
      error: 'Stale search failure',
    },
  ]
  for (const action of staleActions) {
    assert.equal(requestQueueViewReducer(refreshed, action), refreshed)
  }
})

test('a newly delivered identical snapshot discards pagination state', () => {
  const initial = createRequestQueueViewState(queuePages())
  const paginated = requestQueueViewReducer(initial, {
    type: 'load_succeeded',
    generation: initial.generation,
    section: 'open',
    page: page(['open-2'], null),
  })
  assert.deepEqual(
    paginated.pages.open.requests.map(({ id }) => id),
    ['open-1', 'open-2'],
  )

  const identicalSnapshot = queuePages()
  const refreshed = requestQueueViewReducer(paginated, {
    type: 'loader_snapshot_received',
    pages: identicalSnapshot,
  })

  assert.equal(refreshed.generation, paginated.generation + 1)
  assert.equal(refreshed.snapshot, identicalSnapshot)
  assert.deepEqual(
    refreshed.pages.open.requests.map(({ id }) => id),
    ['open-1'],
  )
})

function request(id: string) {
  return { id } as RequestListItem
}

function page(ids: string[], nextCursor: string | null): RequestList {
  return {
    requests: ids.map(request),
    next_cursor: nextCursor,
  }
}

function queuePages(
  overrides: Partial<RequestQueuePages> = {},
): RequestQueuePages {
  return {
    your_work: page(['work-1'], null),
    open: page(['open-1'], 'open:page-2'),
    closed: page(['closed-1'], null),
    ...overrides,
  }
}
