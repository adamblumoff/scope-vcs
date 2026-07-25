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
    section: 'ready',
  })
  assert.equal(loading.loadingSection, 'ready')

  const loaded = requestQueueViewReducer(loading, {
    type: 'load_succeeded',
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
      section: 'completed',
    }),
    {
      type: 'load_failed',
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
  const searching = requestQueueViewReducer(initial, { type: 'search_started' })
  assert.equal(searching.searching, true)
  assert.equal(searching.searchError, null)

  const searched = requestQueueViewReducer(searching, {
    type: 'search_succeeded',
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
    requestQueueViewReducer(searched, { type: 'search_started' }),
    { type: 'search_failed', error: 'Search unavailable.' },
  )
  assert.equal(failed.searching, false)
  assert.equal(failed.searchError, 'Search unavailable.')
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

function queuePages(): RequestQueuePages {
  return {
    your_work: page(['work-1'], null),
    ready: page(['ready-1'], 'ready:page-2'),
    completed: page(['completed-1'], null),
  }
}
