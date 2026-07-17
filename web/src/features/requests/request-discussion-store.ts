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
  appendDiscussionPage,
  applyDiscussionChangesWithoutReordering,
  collectionFromPage,
  type DiscussionCollection,
  insertOptimisticDiscussion,
  markDiscussionFailed,
  markDiscussionRead,
  patchDiscussionForFilter,
  replaceDiscussion,
  orderedDiscussions,
} from './request-discussion-model'
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
  RequestDiscussionFilter,
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionSort,
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
  filter,
  initialPage,
  params,
  repoId,
  sort,
}: {
  actions: RequestDiscussionActions
  actor: RequestActorSummary
  filter: RequestDiscussionFilter
  initialPage: RequestDiscussionPage
  params: RequestParams
  repoId: string
  sort: RequestDiscussionSort
}) {
  const key = requestDiscussionCacheKey({
    filter,
    repoId,
    requestId: params.request_id,
    sort,
  })
  const [collection, setCollection] = useState(() =>
    collectionWithCachedUi(initialPage, readRequestDiscussionCache(key)),
  )
  const [error, setError] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [newActivity, setNewActivity] = useState(false)
  const collectionRef = useRef(collection)
  const catchUpInFlight = useRef(false)
  const catchUpLagged = useRef(false)
  const catchUpTarget = useRef(collection.snapshotVersion)

  useEffect(() => {
    collectionRef.current = collection
  }, [collection])

  const updateCollection = useCallback(
    (update: (current: DiscussionCollection) => DiscussionCollection) => {
      setCollection((current) => {
        const next = update(current)
        collectionRef.current = next
        return next
      })
    },
    [],
  )

  useEffect(() => {
    const authoritative = collectionWithCachedUi(
      initialPage,
      readRequestDiscussionCache(key),
    )
    collectionRef.current = authoritative
    catchUpTarget.current = authoritative.snapshotVersion
    setCollection(authoritative)
    setError(null)
  }, [initialPage, key])

  useEffect(() => {
    writeRequestDiscussionCache(key, collection)
  }, [collection, key])

  const requestQuery = useMemo(
    () => ({
      ...params,
      sort,
      status: filter,
    }),
    [filter, params, sort],
  )

  const refresh = useCallback(async () => {
    setRefreshing(true)
    setError(null)
    try {
      const page = await actions.load(requestQuery)
      updateCollection((current) => {
        const refreshed = collectionWithCachedUi(page, current)
        return refreshed
      })
      setNewActivity(false)
    } catch (requestError) {
      setError(messageFor(requestError, 'Discussions could not be refreshed.'))
    } finally {
      setRefreshing(false)
    }
  }, [actions, requestQuery, updateCollection])

  const catchUp = useCallback(async () => {
    if (catchUpInFlight.current) {
      return
    }
    catchUpInFlight.current = true
    let failed = false
    try {
      while (true) {
        const after = collectionRef.current.snapshotVersion
        const changes = await actions.loadChanges({
          ...params,
          after,
        })
        const progressed = changes.through_position > after
        if (changes.discussions.length > 0) {
          const next = applyDiscussionChangesWithoutReordering(
            collectionRef.current,
            changes.discussions,
            filter,
            changes.through_position,
          )
          collectionRef.current = next
          setCollection(next)
          setNewActivity(true)
        } else if (progressed) {
          const next = {
            ...collectionRef.current,
            snapshotVersion: Math.max(
              collectionRef.current.snapshotVersion,
              changes.through_position,
            ),
          }
          collectionRef.current = next
          setCollection(next)
        }
        const reachedTarget =
          collectionRef.current.snapshotVersion >= catchUpTarget.current
        if (
          !progressed ||
          (reachedTarget && !catchUpLagged.current) ||
          (catchUpLagged.current && changes.discussions.length === 0)
        ) {
          catchUpLagged.current = false
          break
        }
      }
    } catch (requestError) {
      failed = true
      setError(messageFor(requestError, 'New discussion activity could not be loaded.'))
    } finally {
      catchUpInFlight.current = false
      if (
        !failed &&
        collectionRef.current.snapshotVersion < catchUpTarget.current
      ) {
        void catchUp()
      }
    }
  }, [actions, filter, params])

  const onRepoChange = useCallback(
    (event: RepoChangeEvent) => {
      if (event.kind === 'Lagged') {
        catchUpLagged.current = true
        void catchUp()
        return
      }
      if (
        typeof event.kind === 'object' &&
        'RequestDiscussionChanged' in event.kind &&
        event.kind.RequestDiscussionChanged.request_id === params.request_id &&
        event.kind.RequestDiscussionChanged.through_position >
          collectionRef.current.snapshotVersion
      ) {
        catchUpTarget.current = Math.max(
          catchUpTarget.current,
          event.kind.RequestDiscussionChanged.through_position,
        )
        void catchUp()
      }
    },
    [catchUp, params.request_id],
  )
  useRepoChangeSubscription(onRepoChange)
  useEffect(() => {
    void catchUp()
  }, [catchUp, initialPage, key])

  const loadMore = useCallback(async () => {
    if (!collection.nextCursor || loadingMore) return
    setLoadingMore(true)
    setError(null)
    try {
      const page = await actions.load({
        ...requestQuery,
        cursor: collection.nextCursor,
      })
      updateCollection((current) => appendDiscussionPage(current, page))
    } catch (requestError) {
      setError(messageFor(requestError, 'Older discussions could not be loaded.'))
    } finally {
      setLoadingMore(false)
    }
  }, [
    actions,
    collection.nextCursor,
    loadingMore,
    requestQuery,
    updateCollection,
  ])

  const create = useCallback(
    async (
      body: string,
      clientDiscussionId: string = crypto.randomUUID(),
    ) => {
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
        updateCollection((current) =>
          replaceDiscussion(current, result.discussion, clientDiscussionId),
        )
        catchUpTarget.current = Math.max(
          catchUpTarget.current,
          result.discussion.last_activity_position,
        )
        void catchUp()
        return true
      } catch (requestError) {
        updateCollection((current) =>
          markDiscussionFailed(current, clientDiscussionId),
        )
        setError(messageFor(requestError, 'Discussion could not be posted.'))
        return false
      }
    },
    [actions, actor, catchUp, params, updateCollection],
  )

  const retry = useCallback(
    (discussion: RequestDiscussionView) =>
      create(discussion.body_markdown, discussion.id),
    [create],
  )

  const patch = useCallback(
    (discussion: RequestDiscussion) => {
      updateCollection((current) =>
        patchDiscussionForFilter(current, discussion, filter),
      )
      catchUpTarget.current = Math.max(
        catchUpTarget.current,
        discussion.last_activity_position,
      )
      void catchUp()
    },
    [catchUp, filter, updateCollection],
  )

  const markRead = useCallback(
    async (discussion: RequestDiscussion) => {
      if (discussion.unread_count === 0) return
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
        updateCollection((current) => {
          const existing = current.byId.get(discussion.id)
          if (
            !existing ||
            existing.last_activity_position !==
              discussion.last_activity_position
          ) {
            return current
          }
          return replaceDiscussion(current, {
            ...existing,
            unread_count: discussion.unread_count,
          })
        })
      }
    },
    [actions, params, updateCollection],
  )

  const setResolved = useCallback(
    async (discussion: RequestDiscussion, resolved: boolean) => {
      setError(null)
      try {
        const result = await (resolved ? actions.resolve : actions.reopen)({
          ...params,
          discussion_id: discussion.id,
        })
        patch(result.discussion)
      } catch (requestError) {
        setError(
          messageFor(
            requestError,
            resolved
              ? 'Discussion could not be resolved.'
              : 'Discussion could not be reopened.',
          ),
        )
      }
    },
    [actions, params, patch],
  )

  const setExpanded = useCallback(
    (discussionId: string, expanded: boolean) => {
      updateCollection((current) => {
        const discussion = current.byId.get(discussionId)
        return discussion
          ? replaceDiscussion(current, { ...discussion, expanded })
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
    if (cachedDiscussion?.expanded) {
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
