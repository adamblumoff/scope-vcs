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
  pages: RequestQueuePages
  loadingSection: RequestQueueSection | null
  sectionErrors: RequestQueueSectionErrors
  searchDraft: string
  searchQuery: string
  searching: boolean
  searchError: string | null
}

export type RequestQueueViewAction =
  | { type: 'load_started'; section: RequestQueueSection }
  | { type: 'load_succeeded'; section: RequestQueueSection; page: RequestList }
  | { type: 'load_failed'; section: RequestQueueSection; error: string }
  | { type: 'search_draft_changed'; value: string }
  | { type: 'search_started' }
  | {
      type: 'search_succeeded'
      query: string
      ready: RequestList
      completed: RequestList
    }
  | { type: 'search_failed'; error: string }

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
    pages,
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
    case 'load_started':
      return {
        ...state,
        loadingSection: action.section,
        sectionErrors: { ...state.sectionErrors, [action.section]: undefined },
      }
    case 'load_succeeded':
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
      return { ...state, searching: true, searchError: null }
    case 'search_succeeded':
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
      return { ...state, searching: false, searchError: action.error }
  }
}
