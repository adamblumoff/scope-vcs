import type {
  RepoParams,
  RepoRunDetail,
  RepoRunHistoryInput,
  RepoRunHistoryPage,
  RepoRunWorkflowList,
  RunActionInput,
} from '@/api/types'
import { PageContent, WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { RunHistoryList } from './run-history-list'
import { mergeRunHistory, refreshRunHistoryPages } from './run-history-model'
import { RunsHeader } from './runs-header'
import { WorkflowLatestRun } from './workflow-latest-run'
import { WorkflowNavigation } from './workflow-navigation'

const RUNS_REFRESH_INTERVAL_MS = 2_000

type RunPageResources = {
  history: RepoRunHistoryPage
  workflows: RepoRunWorkflowList
  workflowsError: string | null
}

export function RepositoryRunsPage({
  initialResources,
  loadDetail,
  loadHistory,
  params,
  workflow,
}: {
  initialResources: RunPageResources | null
  loadDetail: (input: RunActionInput) => Promise<RepoRunDetail>
  loadHistory: (input: RepoRunHistoryInput) => Promise<RepoRunHistoryPage | null>
  params: RepoParams
  workflow?: string
}) {
  const [history, setHistory] = useState(initialResources?.history ?? null)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const historyRef = useRef(history)
  const loadedPageCountRef = useRef(1)
  const loadingMoreRef = useRef(false)
  const mountedRef = useRef(false)
  const refreshInFlightRef = useRef<Promise<void> | null>(null)
  const { owner, repo } = params
  const input = useMemo(
    () => ({ owner, repo, workflow }),
    [owner, repo, workflow],
  )
  const canRefresh = history !== null

  const refresh = useCallback(() => {
    if (refreshInFlightRef.current) return refreshInFlightRef.current
    if (!historyRef.current || loadingMoreRef.current) return
    const request = refreshRunHistoryPages(
      loadedPageCountRef.current,
      (after) => loadHistory({ ...input, after }),
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
      })
      .finally(() => {
        if (refreshInFlightRef.current === request) {
          refreshInFlightRef.current = null
        }
      })
    refreshInFlightRef.current = request
    return request
  }, [input, loadHistory])

  const loadMore = useCallback(() => {
    if (!history?.next_cursor || loadingMoreRef.current || refreshInFlightRef.current) return
    loadingMoreRef.current = true
    setLoadingMore(true)
    return loadHistory({
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
        if (mountedRef.current) setLoadingMore(false)
      })
  }, [history?.next_cursor, input, loadHistory])

  useEffect(() => {
    historyRef.current = history
  }, [history])

  useEffect(() => {
    mountedRef.current = true
    if (!canRefresh) {
      return () => {
        mountedRef.current = false
      }
    }
    const timer = window.setInterval(
      () => void refresh(),
      RUNS_REFRESH_INTERVAL_MS,
    )
    return () => {
      mountedRef.current = false
      window.clearInterval(timer)
    }
  }, [canRefresh, refresh])

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
      <RunsHeader runCount={history.runs.length} workflowName={selectedWorkflow?.name} />
      <div className="grid min-w-0 border-t border-border lg:grid-cols-[14rem_minmax(0,1fr)]">
        <WorkflowNavigation
          params={params}
          selectedWorkflow={workflow}
          workflows={initialResources.workflows.workflows}
        />
        <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
          {initialResources.workflowsError ? (
            <div className="pt-5">
              <PageErrorAlert title="Workflow navigation unavailable">
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
                  <Button onClick={() => void refresh()} size="sm" variant="secondary">
                    Retry now
                  </Button>
                </div>
              </PageErrorAlert>
            </div>
          ) : null}
          <WorkflowLatestRun
            key={history.runs[0]?.id ?? 'empty'}
            loadDetail={loadDetail}
            params={params}
            run={history.runs[0]}
          />
          <RunHistoryList
            loadMore={() => void loadMore()}
            loadingMore={loadingMore}
            params={params}
            runs={history.runs}
            selectedWorkflowName={selectedWorkflow?.name}
            showLoadMore={history.next_cursor !== null}
          />
        </main>
      </div>
    </WorkbenchPane>
  )
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Run operation failed.'
}
