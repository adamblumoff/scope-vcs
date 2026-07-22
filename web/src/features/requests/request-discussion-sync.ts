import {
  appendDiscussionPage,
  applyDiscussionChanges,
  mergeRefreshedDiscussionPage,
  type DiscussionCollection,
} from './request-discussion-model'
import type {
  RequestDiscussionChanges,
  RequestDiscussionPage,
} from './request-discussion-types'

export type RequestDiscussionSyncOptions = {
  getCollection: () => DiscussionCollection
  getDataGeneration: () => number
  loadChanges: (after: number) => Promise<RequestDiscussionChanges>
  onActivity?: () => void
  onCatchUpError?: (error: unknown) => void
  setCollection: (collection: DiscussionCollection) => void
}

type CatchUpOptions = {
  lagged?: boolean
  target?: number
}

export function createRequestDiscussionSync(
  options: RequestDiscussionSyncOptions,
) {
  let generation = 0
  let key: string | null = null
  let stopped = true
  let target = 0
  let lagged = false
  let inFlight: { generation: number; promise: Promise<void> } | null = null

  const isActive = (issuedGeneration: number) =>
    !stopped && generation === issuedGeneration

  function reset(nextKey: string) {
    if (!stopped && key === nextKey) return
    generation += 1
    key = nextKey
    stopped = false
    target = options.getCollection().snapshotVersion
    lagged = false
    inFlight = null
  }

  function stop() {
    generation += 1
    key = null
    stopped = true
    target = 0
    lagged = false
    inFlight = null
  }

  async function refresh(load: () => Promise<RequestDiscussionPage>) {
    const issuedGeneration = generation
    const baseDataGeneration = options.getDataGeneration()
    let page: RequestDiscussionPage
    try {
      page = await load()
    } catch (error) {
      if (isActive(issuedGeneration)) throw error
      return false
    }
    if (!isActive(issuedGeneration)) return false

    const current = options.getCollection()
    const authoritative =
      options.getDataGeneration() === baseDataGeneration &&
      page.snapshot_version >= current.snapshotVersion
    options.setCollection(
      mergeRefreshedDiscussionPage(current, page, authoritative),
    )
    if (
      !authoritative &&
      page.snapshot_version > current.snapshotVersion
    ) {
      await catchUp({ target: page.snapshot_version })
    }
    return authoritative
  }

  async function paginate(
    cursor: string,
    load: () => Promise<RequestDiscussionPage>,
  ) {
    const issuedGeneration = generation
    let page: RequestDiscussionPage
    try {
      page = await load()
    } catch (error) {
      if (isActive(issuedGeneration)) throw error
      return false
    }
    if (!isActive(issuedGeneration)) return false

    const current = options.getCollection()
    if (current.nextCursor !== cursor) return false
    options.setCollection(appendDiscussionPage(current, page))
    return true
  }

  function catchUp(input: CatchUpOptions = {}) {
    if (stopped) return Promise.resolve()
    if (input.target !== undefined) {
      target = Math.max(target, input.target)
    }
    if (input.lagged) lagged = true
    if (inFlight?.generation === generation) return inFlight.promise

    const issuedGeneration = generation
    const promise = drain(issuedGeneration)
    inFlight = { generation: issuedGeneration, promise }
    void promise.finally(() => {
      if (inFlight?.promise === promise) inFlight = null
    })
    return promise
  }

  async function drain(issuedGeneration: number) {
    try {
      while (isActive(issuedGeneration)) {
        const after = options.getCollection().snapshotVersion
        const changes = await options.loadChanges(after)
        if (!isActive(issuedGeneration)) return

        const current = options.getCollection()
        const next = applyDiscussionChanges(
          current,
          changes.discussions,
          changes.through_position,
        )
        options.setCollection(next)
        if (changes.discussions.length > 0) options.onActivity?.()

        const progressed = changes.through_position > after
        if (changes.has_more && progressed) continue
        if (lagged && changes.discussions.length > 0 && progressed) continue
        if (lagged) lagged = false
        if (next.snapshotVersion < target && progressed) continue
        return
      }
    } catch (error) {
      if (isActive(issuedGeneration)) options.onCatchUpError?.(error)
    }
  }

  return { catchUp, paginate, refresh, reset, stop }
}
