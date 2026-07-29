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
    next_cursor: 'ready:page-2',
  }
  const incoming: RequestList = {
    requests: [first, second],
    next_cursor: 'ready:page-3',
  }

  assert.deepEqual(appendQueuePage(current, incoming), {
    requests: [first, second],
    next_cursor: 'ready:page-3',
  })
  assert.equal(current.next_cursor, 'ready:page-2')
})

test('request queue reducer exposes loading success and error states', () => {
  const initial = createRequestQueueViewState(queuePages())
  const loading = requestQueueViewReducer(initial, {
    type: 'load_started',
    generation: initial.generation,
    section: 'ready',
  })
  assert.equal(loading.loadingSection, 'ready')

  const loaded = requestQueueViewReducer(loading, {
    type: 'load_succeeded',
    generation: loading.generation,
    section: 'ready',
    page: page(['ready-1', 'ready-2'], null),
  })
  assert.equal(loaded.loadingSection, null)
  assert.deepEqual(
    loaded.pages.ready.requests.map(({ id }) => id),
    ['ready-1', 'ready-2'],
  )

  const failed = requestQueueViewReducer(
    requestQueueViewReducer(loaded, {
      type: 'load_started',
      generation: loaded.generation,
      section: 'completed',
    }),
    {
      type: 'load_failed',
      generation: loaded.generation,
      section: 'completed',
      error: 'Could not load completed requests.',
    },
  )
  assert.equal(failed.loadingSection, null)
  assert.equal(
    failed.sectionErrors.completed,
    'Could not load completed requests.',
  )
})

test('request queue reducer replaces searched sections and preserves Your work', () => {
  const initial = {
    ...createRequestQueueViewState(queuePages()),
    searchError: 'Previous failure',
    sectionErrors: {
      ready: 'Ready failure',
      completed: 'Completed failure',
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
    ready: page(['ready-search'], null),
    completed: page(['completed-search'], null),
  })
  assert.equal(searched.searching, false)
  assert.equal(searched.searchQuery, 'needle')
  assert.deepEqual(
    searched.pages.your_work.requests.map(({ id }) => id),
    ['work-1'],
  )
  assert.deepEqual(
    searched.pages.ready.requests.map(({ id }) => id),
    ['ready-search'],
  )
  assert.deepEqual(
    searched.pages.completed.requests.map(({ id }) => id),
    ['completed-search'],
  )
  assert.equal(searched.sectionErrors.ready, undefined)
  assert.equal(searched.sectionErrors.completed, undefined)

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
    loadingSection: 'ready' as const,
    searchDraft: 'old query',
    searchQuery: 'old query',
    searching: true,
  }
  const replacement = queuePages({
    ready: page(['ready-new'], null),
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
      section: 'ready',
      page: page(['ready-stale'], null),
    },
    {
      type: 'load_failed',
      generation: initial.generation,
      section: 'ready',
      error: 'Stale load failure',
    },
    {
      type: 'search_succeeded',
      generation: initial.generation,
      query: 'stale',
      ready: page(['ready-stale'], null),
      completed: page(['completed-stale'], null),
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
    section: 'ready',
    page: page(['ready-2'], null),
  })
  assert.deepEqual(
    paginated.pages.ready.requests.map(({ id }) => id),
    ['ready-1', 'ready-2'],
  )

  const identicalSnapshot = queuePages()
  const refreshed = requestQueueViewReducer(paginated, {
    type: 'loader_snapshot_received',
    pages: identicalSnapshot,
  })

  assert.equal(refreshed.generation, paginated.generation + 1)
  assert.equal(refreshed.snapshot, identicalSnapshot)
  assert.deepEqual(
    refreshed.pages.ready.requests.map(({ id }) => id),
    ['ready-1'],
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
    ready: page(['ready-1'], 'ready:page-2'),
    completed: page(['completed-1'], null),
    ...overrides,
  }
}
