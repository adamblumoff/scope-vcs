import type {
  RepoParams,
  RepoRunHistoryInput,
  RepoRunHistoryPage,
  RepoRunWorkflowList,
} from '@/api/types'
import { PageContent, WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { RunHistoryList } from './run-history-list'
import { mergeRunHistory, reloadRunHistoryPages } from './run-history-model'
import { useRunLiveRefresh } from './run-live-refresh'
import { RunsFilterBar } from './runs-filter-bar'
import {
  type RunStatusFilter,
  runMatchesStatusFilter,
} from './runs-filter-model'

const HISTORY_CHANGES = ['Created', 'StatusChanged'] as const

type RunPageResources = {
  history: RepoRunHistoryPage
  workflows: RepoRunWorkflowList
  workflowsError: string | null
}

export function RepositoryRunsPage({
  initialResources,
  loadHistory,
  params,
  workflow,
}: {
  initialResources: RunPageResources | null
  loadHistory: (
    input: RepoRunHistoryInput,
    signal?: AbortSignal,
  ) => Promise<RepoRunHistoryPage | null>
  params: RepoParams
  workflow?: string
}) {
  const [history, setHistory] = useState(initialResources?.history ?? null)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const [statusFilter, setStatusFilter] = useState<RunStatusFilter>('any')
  const historyRef = useRef(history)
  const loadedPageCountRef = useRef(1)
  const loadingMoreRef = useRef(false)
  const loadMoreInFlightRef = useRef<Promise<void> | null>(null)
  const refreshAfterLoadMoreRef = useRef(false)
  const mountedRef = useRef(false)
  const refreshInFlightRef = useRef<Promise<void> | null>(null)
  const { owner, repo } = params
  const input = useMemo(
    () => ({ owner, repo, workflow }),
    [owner, repo, workflow],
  )
  const canRefresh = history !== null
  const filteredRuns = useMemo(
    () => history
      ? history.runs.filter((run) => runMatchesStatusFilter(run, statusFilter))
      : [],
    [history, statusFilter],
  )

  const refresh = useCallback((signal?: AbortSignal): Promise<void> | undefined => {
    if (refreshInFlightRef.current) return refreshInFlightRef.current
    if (loadMoreInFlightRef.current) {
      refreshAfterLoadMoreRef.current = true
      return
    }
    if (!historyRef.current) return
    const request = reloadRunHistoryPages(
      loadedPageCountRef.current,
      (after) => loadHistory({ ...input, after }, signal),
    )
      .then((next) => {
        if (!mountedRef.current) return
        if (!next) {
          setHistory(null)
          setRefreshError(null)
          return
        }
        setHistory(next)
        setRefreshError(null)
      })
      .catch((error: unknown) => {
        if (mountedRef.current) setRefreshError(errorMessage(error))
        throw error
      })
      .finally(() => {
        if (refreshInFlightRef.current === request) {
          refreshInFlightRef.current = null
        }
      })
    refreshInFlightRef.current = request
    return request
  }, [input, loadHistory])

  const refreshRuns = useRunLiveRefresh({
    acceptedChanges: HISTORY_CHANGES,
    mutable: canRefresh,
    refresh: async (_reasons, signal) => {
      await refresh(signal)
    },
  })

  const loadMore = useCallback(() => {
    if (!history?.next_cursor || loadingMoreRef.current || refreshInFlightRef.current) return
    loadingMoreRef.current = true
    setLoadingMore(true)
    const request = loadHistory({
        ...input,
        after: history.next_cursor,
      })
      .then((next) => {
        if (!mountedRef.current) return
        if (!next) {
          setHistory(null)
          setRefreshError(null)
          return
        }
        setHistory((current) => current
          ? {
              next_cursor: next.next_cursor,
              runs: mergeRunHistory(current.runs, next.runs),
            }
          : next)
        loadedPageCountRef.current += 1
        setRefreshError(null)
      })
      .catch((error: unknown) => {
        if (mountedRef.current) setRefreshError(errorMessage(error))
      })
      .finally(() => {
        loadingMoreRef.current = false
        if (loadMoreInFlightRef.current === request) {
          loadMoreInFlightRef.current = null
        }
        if (mountedRef.current) setLoadingMore(false)
        if (refreshAfterLoadMoreRef.current) {
          refreshAfterLoadMoreRef.current = false
          refreshRuns()
        }
      })
    loadMoreInFlightRef.current = request
    return request
  }, [history?.next_cursor, input, loadHistory, refreshRuns])

  useEffect(() => {
    historyRef.current = history
  }, [history])

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  if (!initialResources || !history) {
    return (
      <PageContent>
        <h1 className="sr-only">Runs</h1>
        <PageErrorAlert title="Runs unavailable">
          Sign in as the owner or a repository member to view runs.
        </PageErrorAlert>
      </PageContent>
    )
  }

  const selectedWorkflow = workflow
    ? initialResources.workflows.workflows.find((item) => item.key === workflow)
    : undefined

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <RunsFilterBar
            onStatusFilterChange={setStatusFilter}
            params={params}
            selectedWorkflow={workflow}
            statusFilter={statusFilter}
            workflows={initialResources.workflows.workflows}
          />
        )}
        title="Runs"
      />
      <div className="min-w-0 border-t border-border">
        <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
          {initialResources.workflowsError ? (
            <div className="pt-5">
              <PageErrorAlert title="Workflow filter unavailable">
                <div>
                  <p>Run history is still available.</p>
                  <p className="mt-1 text-xs">{initialResources.workflowsError}</p>
                </div>
              </PageErrorAlert>
            </div>
          ) : null}
          {refreshError ? (
            <div className="pt-5">
              <PageErrorAlert title="Runs could not refresh">
                <div className="flex flex-wrap items-center gap-3">
                  <span>{refreshError}</span>
                  <Button onClick={refreshRuns} size="sm" variant="secondary">
                    Retry now
                  </Button>
                </div>
              </PageErrorAlert>
            </div>
          ) : null}
          <div className="pt-7">
            <RunHistoryList
              loadMore={() => void loadMore()}
              loadingMore={loadingMore}
              params={params}
              runs={filteredRuns}
              selectedWorkflowName={selectedWorkflow?.name}
              showLoadMore={history.next_cursor !== null}
              totalRunCount={history.runs.length}
            />
          </div>
        </main>
      </div>
    </WorkbenchPane>
  )
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Run operation failed.'
}
