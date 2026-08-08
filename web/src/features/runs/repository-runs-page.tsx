import type {
  RepoParams,
  RepoRunHistoryInput,
  RepoRunHistoryPage,
  RepoRunWorkflowList,
} from '@/api/types'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorContent } from '@/components/route-error-page'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import {
  ArrowRight,
  GitBranch,
  LoaderCircle,
  TerminalSquare,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { formatRunRunnerSelection, formatRunUnixTime } from './run-formatting'

const RUNS_REFRESH_INTERVAL_MS = 2_000

type RunPageResources = {
  history: RepoRunHistoryPage
  workflows: RepoRunWorkflowList
}

export function RepositoryRunsPage({
  initialResources,
  loadHistory,
  params,
  workflow,
}: {
  initialResources: RunPageResources | null
  loadHistory: (input: RepoRunHistoryInput) => Promise<RepoRunHistoryPage | null>
  params: RepoParams
  workflow?: string
}) {
  const [history, setHistory] = useState(initialResources?.history ?? null)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const mountedRef = useRef(false)
  const refreshInFlightRef = useRef<Promise<void> | null>(null)
  const loadedMoreRef = useRef(false)
  const { owner, repo } = params
  const input = useMemo(
    () => ({ owner, repo, workflow }),
    [owner, repo, workflow],
  )
  const canRefresh = history !== null

  const refresh = useCallback(() => {
    if (refreshInFlightRef.current) return refreshInFlightRef.current
    const request = loadHistory(input)
      .then((next) => {
        if (!mountedRef.current) return
        if (!next) {
          setHistory(null)
          setRefreshError(null)
          return
        }
        setHistory((current) => ({
          next_cursor: loadedMoreRef.current
            ? current?.next_cursor ?? next.next_cursor
            : next.next_cursor,
          runs: mergeRunHistory(next.runs, current?.runs ?? []),
        }))
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
    if (!history?.next_cursor || loadingMore) return
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
        loadedMoreRef.current = true
        setHistory((current) => current
          ? {
              next_cursor: next.next_cursor,
              runs: mergeRunHistory(current.runs, next.runs),
            }
          : next)
        setRefreshError(null)
      })
      .catch((error: unknown) => {
        if (mountedRef.current) setRefreshError(errorMessage(error))
      })
      .finally(() => {
        if (mountedRef.current) setLoadingMore(false)
      })
  }, [history?.next_cursor, input, loadHistory, loadingMore])

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
      <>
        <RunsHeader />
        <div className="px-4 pb-12 sm:px-6 lg:px-8">
          <PageErrorAlert title="Runs unavailable">
            Sign in as the owner or a repository member to view runs.
          </PageErrorAlert>
        </div>
      </>
    )
  }

  const selectedWorkflow = workflow
    ? initialResources.workflows.workflows.find((item) => item.key === workflow)
    : undefined

  return (
    <>
      <RunsHeader runCount={history.runs.length} workflowName={selectedWorkflow?.name} />
      <div className="grid min-w-0 border-t border-border lg:grid-cols-[14rem_minmax(0,1fr)]">
        <WorkflowNavigation
          params={params}
          selectedWorkflow={workflow}
          workflows={initialResources.workflows.workflows}
        />
        <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
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
          <RunHistory
            loadMore={() => void loadMore()}
            loadingMore={loadingMore}
            params={params}
            runs={history.runs}
            selectedWorkflowName={selectedWorkflow?.name}
            showLoadMore={history.next_cursor !== null}
          />
        </main>
      </div>
    </>
  )
}

function WorkflowNavigation({
  params,
  selectedWorkflow,
  workflows,
}: {
  params: RepoParams
  selectedWorkflow?: string
  workflows: RepoRunWorkflowList['workflows']
}) {
  const baseClass = 'flex shrink-0 items-center gap-2.5 whitespace-nowrap rounded-md px-2.5 py-2 text-sm outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring lg:w-full'
  return (
    <nav
      aria-label="Run workflows"
      className="flex min-w-0 gap-1 overflow-x-auto border-b border-border px-3 py-3 lg:block lg:min-h-[32rem] lg:overflow-visible lg:border-b-0 lg:border-r lg:px-3 lg:py-6"
    >
      <p className="hidden px-2.5 pb-2 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground lg:block">
        Workflows
      </p>
      <Link
        activeOptions={{ exact: true }}
        className={cn(baseClass, !selectedWorkflow && 'bg-muted font-medium text-foreground')}
        params={params}
        to="/$owner/$repo/runs"
      >
        <GitBranch className="size-3.5" />
        All workflows
      </Link>
      {workflows.map((item) => (
        <Link
          className={cn(
            baseClass,
            selectedWorkflow === item.key && 'bg-muted font-medium text-foreground',
          )}
          key={item.key}
          params={{ ...params, workflow: item.key }}
          to="/$owner/$repo/runs/workflows/$workflow"
        >
          <span className="size-2 rounded-full border-2 border-current" />
          <span className="max-w-44 truncate">{item.name}</span>
        </Link>
      ))}
    </nav>
  )
}

function RunHistory({
  loadMore,
  loadingMore,
  params,
  runs,
  selectedWorkflowName,
  showLoadMore,
}: {
  loadMore: () => void
  loadingMore: boolean
  params: RepoParams
  runs: RepoRunHistoryPage['runs']
  selectedWorkflowName?: string
  showLoadMore: boolean
}) {
  return (
    <section aria-labelledby="run-history-heading" className="pt-7 lg:pt-9">
      <div className="flex items-center gap-2">
        <TerminalSquare className="size-4 text-muted-foreground" />
        <h2 className="text-sm font-semibold" id="run-history-heading">
          {selectedWorkflowName ? `${selectedWorkflowName} history` : 'All workflow runs'}
        </h2>
        <span className="text-xs tabular-nums text-muted-foreground">
          {runs.length}
        </span>
      </div>
      <div className="mt-3 divide-y divide-border border-y border-border">
        {runs.length === 0 ? (
          <EmptyRunRow selectedWorkflowName={selectedWorkflowName} />
        ) : (
          runs.map((run) => {
            const state = run.cancellation_requested ? 'canceling' : run.state
            return (
              <Link
                className="group flex min-h-16 min-w-0 items-center gap-3 px-2 py-4 outline-none hover:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                key={run.id}
                params={{ ...params, runId: run.id }}
                to="/$owner/$repo/runs/$runId"
              >
                <StatusDot state={run.state} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">
                    {run.workflow_name}
                  </span>
                  <span className="mt-1 block truncate font-mono text-[11px] text-muted-foreground">
                    {run.git_oid.slice(0, 12)} · {formatRunRunnerSelection(run.runner_selection)}
                  </span>
                </span>
                <span className="shrink-0 text-right text-xs text-muted-foreground">
                  <span className="block capitalize text-foreground">{state}</span>
                  <span className="block">{formatRunUnixTime(run.updated_at_unix)}</span>
                </span>
                <ArrowRight className="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
              </Link>
            )
          })
        )}
      </div>
      {showLoadMore ? (
        <div className="flex justify-center pt-5">
          <Button disabled={loadingMore} onClick={loadMore} variant="secondary">
            {loadingMore ? <LoaderCircle className="animate-spin" /> : null}
            Load older runs
          </Button>
        </div>
      ) : null}
    </section>
  )
}

function RunsHeader({
  runCount,
  workflowName,
}: {
  runCount?: number
  workflowName?: string
}) {
  return (
    <WorkbenchHeader
      actions={(
        <code className="max-w-full overflow-x-auto whitespace-nowrap text-xs text-muted-foreground">
          scope run &lt;workflow&gt; --runner &lt;name&gt;
        </code>
      )}
      count={runCount === undefined ? undefined : `${runCount} loaded`}
      description={(
        <>
          {workflowName ? `History for ${workflowName}. ` : null}
          Jobs from <code>.scope/runs</code> on your attached machines.
        </>
      )}
      eyebrow="Execute"
      title="Runs"
    />
  )
}

export function RunsPagePending() {
  return (
    <>
      <RunsHeader />
      <output
        aria-busy="true"
        className="flex items-center gap-2 border-t border-border px-4 py-10 text-sm text-muted-foreground sm:px-6 lg:px-8"
      >
        <LoaderCircle className="size-4 animate-spin" />
        Loading workflows and runs
      </output>
    </>
  )
}

export function RunsPageError({ error }: { error: unknown }) {
  return (
    <>
      <RunsHeader />
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected runs error"
        title="Runs unavailable"
      />
    </>
  )
}

function EmptyRunRow({ selectedWorkflowName }: { selectedWorkflowName?: string }) {
  return (
    <div className="px-2 py-7">
      <p className="text-sm font-medium">
        {selectedWorkflowName ? `No ${selectedWorkflowName} runs yet` : 'No runs yet'}
      </p>
      <p className="mt-1 text-sm text-muted-foreground">
        Push to main with a matching trigger, or run a workflow manually from the CLI.
      </p>
    </div>
  )
}

export function RunStatusDot({
  className,
  state,
}: {
  className?: string
  state: string
}) {
  return <StatusDot className={className} state={state} />
}

function StatusDot({ className, state }: { className?: string; state: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'size-2 shrink-0 rounded-full',
        ['online', 'running', 'succeeded'].includes(state) && 'bg-emerald-500',
        ['blocked', 'queued', 'leased', 'pending'].includes(state) && 'bg-amber-500',
        ['failed', 'lost', 'offline'].includes(state) && 'bg-destructive',
        ['canceled', 'disabled', 'skipped'].includes(state) && 'bg-muted-foreground',
        className,
      )}
    />
  )
}

function mergeRunHistory(
  first: RepoRunHistoryPage['runs'],
  second: RepoRunHistoryPage['runs'],
) {
  const seen = new Set(first.map((run) => run.id))
  return [...first, ...second.filter((run) => !seen.has(run.id))]
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Run operation failed.'
}
