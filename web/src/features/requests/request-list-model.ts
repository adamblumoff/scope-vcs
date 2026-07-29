import type { RequestList, RequestListItem } from '@/api/types'
import type { RequestQueueSection } from '@/api/request-queue-input'

export type RequestQueuePages = Record<RequestQueueSection, RequestList>

export const REQUEST_QUEUE_SECTION_ORDER = [
  'your_work',
  'ready',
  'completed',
] as const satisfies readonly RequestQueueSection[]

export type RequestQueueSectionErrors = Partial<
  Record<RequestQueueSection, string>
>

export type RequestQueueViewState = {
  generation: number
  pages: RequestQueuePages
  snapshot: RequestQueuePages
  loadingSection: RequestQueueSection | null
  sectionErrors: RequestQueueSectionErrors
  searchDraft: string
  searchQuery: string
  searching: boolean
  searchError: string | null
}

export type RequestQueueViewAction =
  | { type: 'loader_snapshot_received'; pages: RequestQueuePages }
  | {
      type: 'load_started'
      generation: number
      section: RequestQueueSection
    }
  | {
      type: 'load_succeeded'
      generation: number
      section: RequestQueueSection
      page: RequestList
    }
  | {
      type: 'load_failed'
      generation: number
      section: RequestQueueSection
      error: string
    }
  | { type: 'search_draft_changed'; value: string }
  | { type: 'search_started'; generation: number }
  | {
      type: 'search_succeeded'
      generation: number
      query: string
      ready: RequestList
      completed: RequestList
    }
  | { type: 'search_failed'; generation: number; error: string }

export function appendRequestPage(
  current: RequestListItem[],
  incoming: RequestListItem[],
) {
  const knownIds = new Set(current.map((request) => request.id))
  const additions = incoming.filter((request) => {
    if (knownIds.has(request.id)) {
      return false
    }
    knownIds.add(request.id)
    return true
  })
  return [...current, ...additions]
}

export function appendQueuePage(
  current: RequestList,
  incoming: RequestList,
): RequestList {
  return {
    requests: appendRequestPage(current.requests, incoming.requests),
    next_cursor: incoming.next_cursor,
  }
}

export function requestCountLabel(count: number, hasMore: boolean) {
  const suffix = count === 1 && !hasMore ? 'request' : 'requests'
  return `${count}${hasMore ? '+' : ''} ${suffix}`
}

export function createRequestQueueViewState(
  pages: RequestQueuePages,
): RequestQueueViewState {
  return {
    generation: 0,
    pages,
    snapshot: pages,
    loadingSection: null,
    sectionErrors: {},
    searchDraft: '',
    searchQuery: '',
    searching: false,
    searchError: null,
  }
}

export function requestQueueViewReducer(
  state: RequestQueueViewState,
  action: RequestQueueViewAction,
): RequestQueueViewState {
  switch (action.type) {
    case 'loader_snapshot_received':
      return {
        ...createRequestQueueViewState(action.pages),
        generation: state.generation + 1,
      }
    case 'load_started':
      if (action.generation !== state.generation) return state
      return {
        ...state,
        loadingSection: action.section,
        sectionErrors: { ...state.sectionErrors, [action.section]: undefined },
      }
    case 'load_succeeded':
      if (action.generation !== state.generation) return state
      return {
        ...state,
        loadingSection: null,
        pages: {
          ...state.pages,
          [action.section]: appendQueuePage(
            state.pages[action.section],
            action.page,
          ),
        },
      }
    case 'load_failed':
      if (action.generation !== state.generation) return state
      return {
        ...state,
        loadingSection: null,
        sectionErrors: {
          ...state.sectionErrors,
          [action.section]: action.error,
        },
      }
    case 'search_draft_changed':
      return { ...state, searchDraft: action.value }
    case 'search_started':
      if (action.generation !== state.generation) return state
      return { ...state, searching: true, searchError: null }
    case 'search_succeeded':
      if (action.generation !== state.generation) return state
      return {
        ...state,
        pages: {
          ...state.pages,
          completed: action.completed,
          ready: action.ready,
        },
        searchQuery: action.query,
        searching: false,
        searchError: null,
        sectionErrors: {
          ...state.sectionErrors,
          completed: undefined,
          ready: undefined,
        },
      }
    case 'search_failed':
      if (action.generation !== state.generation) return state
      return { ...state, searching: false, searchError: action.error }
  }
}
