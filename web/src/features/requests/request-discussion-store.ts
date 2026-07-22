import type { RequestParams } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import { useRepoChangeSubscription } from '@/features/repo-detail/repo-layout-context'
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  readRequestDiscussionCache,
  requestDiscussionCacheKey,
  writeRequestDiscussionCache,
} from './request-discussion-cache'
import {
  collectionFromPage,
  type DiscussionCollection,
  insertOptimisticDiscussion,
  markDiscussionFailed,
  markDiscussionRead,
  mergeDiscussion,
  orderedDiscussions,
  reconcileDiscussionMutation,
} from './request-discussion-model'
import { createRequestDiscussionSync } from './request-discussion-sync'
import type {
  CreateDiscussionInput,
  LoadDiscussionsInput,
  MarkDiscussionReadInput,
  RequestDiscussionActionInput,
} from './request-discussion-api'
import type {
  RequestActorSummary,
  RequestDiscussion,
  RequestDiscussionChanges,
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionView,
} from './request-discussion-types'

export type RequestDiscussionActions = {
  create: (input: CreateDiscussionInput) => Promise<RequestDiscussionMutation>
  load: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  loadChanges: (
    input: RequestParams & { after: number },
  ) => Promise<RequestDiscussionChanges>
  markRead: (input: MarkDiscussionReadInput) => Promise<unknown>
  reopen: (
    input: RequestDiscussionActionInput,
  ) => Promise<RequestDiscussionMutation>
  resolve: (
    input: RequestDiscussionActionInput,
  ) => Promise<RequestDiscussionMutation>
}

export function useRequestDiscussionStore({
  actions,
  actor,
  initialPage,
  params,
  repoId,
}: {
  actions: RequestDiscussionActions
  actor: RequestActorSummary
  initialPage: RequestDiscussionPage
  params: RequestParams
  repoId: string
}) {
  const key = requestDiscussionCacheKey({
    repoId,
    requestId: params.request_id,
  })
  const [collection, setCollection] = useState(() =>
    collectionWithCachedUi(initialPage, readRequestDiscussionCache(key)),
  )
  const [error, setError] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [newActivity, setNewActivity] = useState(false)
  const collectionRef = useRef(collection)
  const activeKeyRef = useRef(key)

  const updateCollection = useCallback(
    (update: (current: DiscussionCollection) => DiscussionCollection) => {
      const next = update(collectionRef.current)
      collectionRef.current = next
      setCollection(next)
    },
    [],
  )

  const setCurrentCollection = useCallback((next: DiscussionCollection) => {
    collectionRef.current = next
    setCollection(next)
  }, [])

  const sync = useMemo(
    () =>
      createRequestDiscussionSync({
        getCollection: () => collectionRef.current,
        loadChanges: (after) => actions.loadChanges({ ...params, after }),
        onActivity: () => setNewActivity(true),
        onCatchUpError: (requestError) => {
          setError(
            messageFor(
              requestError,
              'New discussion activity could not be loaded.',
            ),
          )
        },
        setCollection: setCurrentCollection,
      }),
    [actions, params, setCurrentCollection],
  )
  const currentSyncRef = useRef(sync)
  currentSyncRef.current = sync
  const isCurrent = useCallback(
    (operationKey: string, operationSync: typeof sync) =>
      activeKeyRef.current === operationKey &&
      currentSyncRef.current === operationSync,
    [],
  )

  useEffect(() => {
    const keyChanged = activeKeyRef.current !== key
    activeKeyRef.current = key
    if (keyChanged) {
      setCurrentCollection(
        collectionWithCachedUi(
          initialPage,
          readRequestDiscussionCache(key),
        ),
      )
      setError(null)
      setLoadingMore(false)
      setRefreshing(false)
      setNewActivity(false)
    }
    sync.reset(key)

    let cancelled = false
    async function initialize() {
      if (!keyChanged) {
        const authoritative = await sync.refresh(() =>
          Promise.resolve(initialPage),
        )
        if (
          !cancelled &&
          authoritative &&
          isCurrent(key, sync)
        ) {
          setNewActivity(false)
        }
      }
      await sync.catchUp()
    }
    void initialize()

    return () => {
      cancelled = true
      sync.stop()
    }
  }, [initialPage, isCurrent, key, setCurrentCollection, sync])

  useEffect(() => {
    writeRequestDiscussionCache(key, collectionRef.current)
  }, [collection, key])

  const refresh = useCallback(async () => {
    const operationKey = key
    setRefreshing(true)
    setError(null)
    try {
      const authoritative = await sync.refresh(() => actions.load(params))
      if (authoritative && isCurrent(operationKey, sync)) {
        setNewActivity(false)
      }
    } catch (requestError) {
      if (isCurrent(operationKey, sync)) {
        setError(messageFor(requestError, 'Discussions could not be refreshed.'))
      }
    } finally {
      if (isCurrent(operationKey, sync)) {
        setRefreshing(false)
      }
    }
  }, [actions, isCurrent, key, params, sync])

  const onRepoChange = useCallback(
    (event: RepoChangeEvent) => {
      if (event.kind === 'Lagged') {
        void sync.catchUp({ lagged: true })
        return
      }
      if (
        typeof event.kind === 'object' &&
        'RequestTimelineChanged' in event.kind &&
        event.kind.RequestTimelineChanged.request_id === params.request_id &&
        event.kind.RequestTimelineChanged.through_position >
          collectionRef.current.snapshotVersion
      ) {
        void sync.catchUp({
          target: event.kind.RequestTimelineChanged.through_position,
        })
      }
    },
    [params.request_id, sync],
  )
  useRepoChangeSubscription(onRepoChange)

  const loadMore = useCallback(async () => {
    const cursor = collection.nextCursor
    if (!cursor || loadingMore) return
    const operationKey = key
    setLoadingMore(true)
    setError(null)
    try {
      await sync.paginate(cursor, () => actions.load({ ...params, cursor }))
    } catch (requestError) {
      if (isCurrent(operationKey, sync)) {
        setError(messageFor(requestError, 'Older discussions could not be loaded.'))
      }
    } finally {
      if (isCurrent(operationKey, sync)) {
        setLoadingMore(false)
      }
    }
  }, [
    actions,
    collection.nextCursor,
    isCurrent,
    key,
    loadingMore,
    params,
    sync,
  ])

  const create = useCallback(
    async (
      body: string,
      clientDiscussionId: string = crypto.randomUUID(),
    ) => {
      const operationKey = key
      const optimistic = optimisticDiscussion({
        actor,
        body,
        clientDiscussionId,
        requestId: params.request_id,
      })
      updateCollection((current) =>
        insertOptimisticDiscussion(current, optimistic),
      )
      setError(null)
      try {
        const result = await actions.create({
          ...params,
          body_markdown: body,
          client_discussion_id: clientDiscussionId,
        })
        if (!isCurrent(operationKey, sync)) {
          return false
        }
        updateCollection((current) =>
          reconcileDiscussionMutation(
            current,
            result.discussion,
            clientDiscussionId,
          ),
        )
        void sync.catchUp({
          target: result.discussion.last_activity_position,
        })
        return true
      } catch (requestError) {
        if (isCurrent(operationKey, sync)) {
          updateCollection((current) =>
            markDiscussionFailed(current, clientDiscussionId),
          )
          setError(messageFor(requestError, 'Discussion could not be posted.'))
        }
        return false
      }
    },
    [actions, actor, isCurrent, key, params, sync, updateCollection],
  )

  const retry = useCallback(
    (discussion: RequestDiscussionView) =>
      discussion.body_markdown
        ? create(discussion.body_markdown, discussion.id)
        : Promise.resolve(false),
    [create],
  )

  const patch = useCallback(
    (discussion: RequestDiscussion) => {
      if (
        !isCurrent(key, sync) ||
        discussion.request_id !== params.request_id
      ) {
        return
      }
      updateCollection((current) => mergeDiscussion(current, discussion))
      void sync.catchUp({ target: discussion.last_activity_position })
    },
    [isCurrent, key, params.request_id, sync, updateCollection],
  )

  const markRead = useCallback(
    async (discussion: RequestDiscussion) => {
      if (discussion.unread_count === 0) return
      const operationKey = key
      updateCollection((current) =>
        markDiscussionRead(current, discussion.id),
      )
      try {
        await actions.markRead({
          ...params,
          discussion_id: discussion.id,
          through_position: discussion.last_activity_position,
        })
      } catch {
        if (!isCurrent(operationKey, sync)) {
          return
        }
        updateCollection((current) => {
          const existing = current.byId.get(discussion.id)
          if (
            !existing ||
            existing.last_activity_position !==
              discussion.last_activity_position
          ) {
            return current
          }
          return mergeDiscussion(current, {
            ...existing,
            unread_count: discussion.unread_count,
          })
        })
      }
    },
    [actions, isCurrent, key, params, sync, updateCollection],
  )

  const setResolved = useCallback(
    async (discussion: RequestDiscussion, resolved: boolean) => {
      const operationKey = key
      setError(null)
      try {
        const result = await (resolved ? actions.resolve : actions.reopen)({
          ...params,
          discussion_id: discussion.id,
        })
        if (isCurrent(operationKey, sync)) {
          patch(result.discussion)
        }
      } catch (requestError) {
        if (isCurrent(operationKey, sync)) {
          setError(
            messageFor(
              requestError,
              resolved
                ? 'Discussion could not be resolved.'
                : 'Discussion could not be reopened.',
            ),
          )
        }
      }
    },
    [actions, isCurrent, key, params, patch, sync],
  )

  const setExpanded = useCallback(
    (discussionId: string, expanded: boolean) => {
      updateCollection((current) => {
        const discussion = current.byId.get(discussionId)
        return discussion
          ? mergeDiscussion(current, { ...discussion, expanded })
          : current
      })
    },
    [updateCollection],
  )

  return {
    cacheKey: key,
    collection,
    create,
    discussions: orderedDiscussions(collection),
    error,
    loadMore,
    loadingMore,
    markRead,
    newActivity,
    patch,
    refresh,
    refreshing,
    retry,
    setExpanded,
    setResolved,
  }
}

function collectionWithCachedUi(
  page: RequestDiscussionPage,
  cached: ReturnType<typeof readRequestDiscussionCache>,
) {
  const collection = collectionFromPage(page)
  if (!cached) return collection
  const byId = new Map(collection.byId)
  for (const [discussionId, discussion] of byId) {
    const cachedDiscussion = cached.byId.get(discussionId)
    if (cachedDiscussion?.expanded && !discussion.change_block) {
      byId.set(discussionId, { ...discussion, expanded: true })
    }
  }
  return { ...collection, byId }
}

function optimisticDiscussion({
  actor,
  body,
  clientDiscussionId,
  requestId,
}: {
  actor: RequestActorSummary
  body: string
  clientDiscussionId: string
  requestId: string
}): RequestDiscussionView {
  const position = Number.MAX_SAFE_INTEGER
  return {
    author: actor,
    body_markdown: body,
    change_block: null,
    created_at_unix: Math.floor(Date.now() / 1000),
    id: clientDiscussionId,
    last_activity_position: position,
    latest_replies: [],
    opened_position: position,
    pending: 'sending',
    reply_count: 0,
    request_id: requestId,
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}

function messageFor(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim()
    ? error.message
    : fallback
}
