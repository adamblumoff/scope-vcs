import assert from 'node:assert/strict'
import test from 'node:test'
import {
  applyDiscussionChanges,
  collectionFromPage,
  reconcileDiscussionMutation,
  type DiscussionCollection,
} from './request-discussion-model'
import { createRequestDiscussionSync } from './request-discussion-sync'
import type {
  RequestDiscussion,
  RequestDiscussionChanges,
  RequestDiscussionPage,
} from './request-discussion-types'

test('late refresh cannot overwrite a newer catch-up', async () => {
  let collection = pageCollection('request-a', 10)
  const refreshPage = deferred<ReturnType<typeof page>>()
  const changes = deferred<RequestDiscussionChanges>()
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => collection.snapshotVersion,
    loadChanges: () => changes.promise,
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  const refreshing = sync.refresh(() => refreshPage.promise)
  const catchingUp = sync.catchUp({ target: 11 })
  changes.resolve(changeBatch([discussion('one', 11, 'request-a')], 11))
  await catchingUp
  refreshPage.resolve(page('request-a', 10))

  assert.equal(await refreshing, false)
  assert.equal(collection.snapshotVersion, 11)
  assert.equal(collection.byId.get('one')?.last_activity_position, 11)
})

test('late refresh cannot discard a concurrent mutation response', async () => {
  let collection = pageCollection('request-a', 10)
  let dataGeneration = 0
  const refreshPage = deferred<ReturnType<typeof page>>()
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => dataGeneration,
    loadChanges: async () => changeBatch([], collection.snapshotVersion),
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  const refreshing = sync.refresh(() => refreshPage.promise)
  collection = reconcileDiscussionMutation(
    collection,
    discussion('created', 11, 'request-a'),
  )
  dataGeneration += 1
  refreshPage.resolve(page('request-a', 10))

  assert.equal(await refreshing, false)
  assert.equal(collection.snapshotVersion, 10)
  assert.equal(collection.byId.get('created')?.last_activity_position, 11)
})

test('non-authoritative refresh drains changes from the prior snapshot', async () => {
  let collection = pageCollection('request-a', 10)
  let dataGeneration = 0
  const refreshPage = deferred<ReturnType<typeof page>>()
  const afters: number[] = []
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => dataGeneration,
    loadChanges: async (after) => {
      afters.push(after)
      return changeBatch([discussion('one', 11, 'request-a')], 12)
    },
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  const refreshing = sync.refresh(() => refreshPage.promise)
  collection = {
    ...collection,
    byId: new Map(collection.byId),
  }
  dataGeneration += 1
  refreshPage.resolve({
    discussions: [discussion('new-root', 12, 'request-a')],
    next_cursor: null,
    snapshot_version: 12,
  })

  assert.equal(await refreshing, false)
  assert.deepEqual(afters, [10])
  assert.equal(collection.snapshotVersion, 12)
  assert.equal(collection.byId.get('one')?.last_activity_position, 11)
  assert.equal(collection.byId.get('new-root')?.last_activity_position, 12)
})

test('UI-only changes do not make a refresh non-authoritative', async () => {
  let collection = pageCollection('request-a', 10)
  let dataGeneration = 0
  const refreshPage = deferred<ReturnType<typeof page>>()
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => dataGeneration,
    loadChanges: async () => changeBatch([], collection.snapshotVersion),
    setCollection: (next) => {
      collection = next
      dataGeneration += 1
    },
  })
  sync.reset('repo-1/request-a')

  const refreshing = sync.refresh(() => refreshPage.promise)
  collection = {
    ...collection,
    byId: new Map(collection.byId),
  }
  refreshPage.resolve({
    discussions: [
      discussion('newer', 11, 'request-a'),
      discussion('one', 10, 'request-a'),
    ],
    next_cursor: 'older',
    snapshot_version: 11,
  })

  assert.equal(await refreshing, true)
  assert.equal(collection.snapshotVersion, 11)
  assert.equal(collection.nextCursor, 'older')
  assert.deepEqual(collection.order, ['one', 'newer'])
})

test('catch-up coalesces a newer target while a request is in flight', async () => {
  let collection = pageCollection('request-a', 5)
  const first = deferred<RequestDiscussionChanges>()
  const afters: number[] = []
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => collection.snapshotVersion,
    loadChanges: (after) => {
      afters.push(after)
      return after === 5
        ? first.promise
        : Promise.resolve(
            changeBatch([discussion('three', 12, 'request-a')], 12),
          )
    },
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  const original = sync.catchUp({ target: 10 })
  const coalesced = sync.catchUp({ target: 12 })
  assert.equal(coalesced, original)
  first.resolve(changeBatch([discussion('two', 10, 'request-a')], 10))
  await original

  assert.deepEqual(afters, [5, 10])
  assert.equal(collection.snapshotVersion, 12)
  assert.deepEqual(collection.order, ['one', 'two', 'three'])
})

test('catch-up drains every server page when continuation is explicit', async () => {
  let collection = pageCollection('request-a', 5)
  const firstPage = Array.from({ length: 100 }, (_, index) =>
    discussion(`discussion-${index + 6}`, index + 6, 'request-a'),
  )
  const responses = [
    changeBatch(firstPage, 105, true),
    changeBatch([discussion('discussion-106', 106, 'request-a')], 106),
  ]
  const afters: number[] = []
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => collection.snapshotVersion,
    loadChanges: async (after) => {
      afters.push(after)
      const response = responses.shift()
      assert.ok(response)
      return response
    },
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  await sync.catchUp()

  assert.deepEqual(afters, [5, 105])
  assert.equal(collection.snapshotVersion, 106)
  assert.equal(collection.order.length, 102)
})

test('lagged catch-up polls until an empty response confirms the head', async () => {
  let collection = pageCollection('request-a', 5)
  const responses = [
    changeBatch([discussion('two', 6, 'request-a')], 6),
    changeBatch([], 7),
  ]
  const afters: number[] = []
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => collection.snapshotVersion,
    loadChanges: async (after) => {
      afters.push(after)
      const response = responses.shift()
      assert.ok(response)
      return response
    },
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  await sync.catchUp({ lagged: true })

  assert.deepEqual(afters, [5, 6])
  assert.equal(collection.snapshotVersion, 7)
  assert.deepEqual(collection.order, ['one', 'two'])
})

test('a completion from the previous request generation is discarded', async () => {
  let collection = pageCollection('request-a', 5)
  const changes = deferred<RequestDiscussionChanges>()
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => collection.snapshotVersion,
    loadChanges: () => changes.promise,
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')
  const oldCatchUp = sync.catchUp({ target: 8 })

  collection = pageCollection('request-b', 20)
  sync.reset('repo-1/request-b')
  changes.resolve(
    changeBatch([discussion('old-request-row', 8, 'request-a')], 8),
  )
  await oldCatchUp

  assert.equal(collection.snapshotVersion, 20)
  assert.deepEqual(collection.order, ['one'])
  assert.equal(collection.byId.has('old-request-row'), false)
})

test('pagination keeps an entity advanced by live catch-up', async () => {
  let collection: DiscussionCollection = {
    ...pageCollection('request-a', 12),
    nextCursor: 'older',
  }
  const olderPage = deferred<ReturnType<typeof page>>()
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => collection.snapshotVersion,
    loadChanges: async () => changeBatch([], collection.snapshotVersion),
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')
  const loading = sync.paginate('older', () => olderPage.promise)

  collection = applyDiscussionChanges(
    collection,
    [discussion('one', 15, 'request-a')],
    15,
  )
  olderPage.resolve({
    discussions: [
      discussion('one', 7, 'request-a'),
      discussion('older', 6, 'request-a'),
    ],
    next_cursor: null,
    snapshot_version: 12,
  })

  assert.equal(await loading, true)
  assert.equal(collection.byId.get('one')?.last_activity_position, 15)
  assert.equal(collection.snapshotVersion, 15)
  assert.equal(collection.nextCursor, null)
  assert.deepEqual(collection.order, ['older', 'one'])
})

test('only a current refresh reports authoritative ordering', async () => {
  let collection: DiscussionCollection = {
    ...pageCollection('request-a', 10),
    order: ['one'],
  }
  const sync = createRequestDiscussionSync({
    getCollection: () => collection,
    getDataGeneration: () => collection.snapshotVersion,
    loadChanges: async () => changeBatch([], collection.snapshotVersion),
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  const authoritative = await sync.refresh(async () => ({
    discussions: [
      discussion('newer', 11, 'request-a'),
      discussion('one', 10, 'request-a'),
    ],
    next_cursor: null,
    snapshot_version: 11,
  }))

  assert.equal(authoritative, true)
  assert.equal(collection.snapshotVersion, 11)
  assert.deepEqual(collection.order, ['one', 'newer'])
})

function pageCollection(requestId: string, position: number) {
  return collectionFromPage(page(requestId, position))
}

function page(requestId: string, position: number): RequestDiscussionPage {
  return {
    discussions: [discussion('one', position, requestId)],
    next_cursor: null,
    snapshot_version: position,
  }
}

function changeBatch(
  discussions: RequestDiscussion[],
  throughPosition: number,
  hasMore = false,
): RequestDiscussionChanges {
  return {
    discussions,
    has_more: hasMore,
    through_position: throughPosition,
  }
}

function discussion(
  id: string,
  lastActivity: number,
  requestId: string,
): RequestDiscussion {
  return {
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: `Discussion ${id}`,
    client_discussion_id: id,
    created_at_unix: lastActivity,
    id,
    last_activity_position: lastActivity,
    latest_replies: [],
    opened_position: lastActivity,
    reply_count: 0,
    request_id: requestId,
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((complete) => {
    resolve = complete
  })
  return { promise, resolve }
}
