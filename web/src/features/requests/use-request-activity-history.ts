import { useCallback, useReducer, useRef } from 'react'
import type { RequestActivityPage } from './request-discussion-types'

type HistoryState = {
  activity: RequestActivityPage | null
  error: string | null
  loading: boolean
  open: boolean
}

type HistoryAction =
  | { type: 'close' }
  | { type: 'failed' }
  | { activity: RequestActivityPage; type: 'loaded' }
  | { open: boolean; type: 'loading' }

const initialState: HistoryState = {
  activity: null,
  error: null,
  loading: false,
  open: false,
}

export function useRequestActivityHistory(
  load: () => Promise<RequestActivityPage>,
) {
  const [state, dispatch] = useReducer(reduceHistory, initialState)
  const loadVersion = useRef(0)

  const loadSnapshot = useCallback((open: boolean) => {
    const version = ++loadVersion.current
    dispatch({ open, type: 'loading' })
    void load().then(
      (activity) => {
        if (version !== loadVersion.current) return
        dispatch({ activity, type: 'loaded' })
      },
      () => {
        if (version !== loadVersion.current) return
        dispatch({ type: 'failed' })
      },
    )
  }, [load])

  const open = useCallback(() => loadSnapshot(true), [loadSnapshot])
  const retry = useCallback(() => loadSnapshot(false), [loadSnapshot])
  const onOpenChange = useCallback((nextOpen: boolean) => {
    if (nextOpen) return
    loadVersion.current += 1
    dispatch({ type: 'close' })
  }, [])

  return { ...state, onOpenChange, openHistory: open, retry }
}

function reduceHistory(
  state: HistoryState,
  action: HistoryAction,
): HistoryState {
  switch (action.type) {
    case 'close':
      return initialState
    case 'failed':
      return { ...state, error: 'Request history could not be loaded.', loading: false }
    case 'loaded':
      return { ...state, activity: action.activity, loading: false }
    case 'loading':
      return {
        activity: null,
        error: null,
        loading: true,
        open: action.open || state.open,
      }
  }
}
