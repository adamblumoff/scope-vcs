import type {
  RepoOperations,
  RepoParams,
  RepoRun,
  RepoRunDetail,
  RunActionInput,
} from '@/api/types'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorContent } from '@/components/route-error-page'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { cn } from '@/lib/utils'
import {
  selectedRunBecameTerminal,
  selectedRunIsUnavailable,
  shouldRefreshSelectedRunDetail,
} from './repository-runs-refresh'
import {
  ChevronDown,
  ChevronRight,
  LoaderCircle,
  RotateCcw,
  Server,
  Square,
  TerminalSquare,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'

const DATE_FORMATTER = new Intl.DateTimeFormat('en-US', {
  dateStyle: 'medium',
  timeZone: 'UTC',
  timeStyle: 'short',
})
const RUNS_REFRESH_INTERVAL_MS = 2_000

export function RepositoryRunsPage({
  cancelRun,
  initialOperations,
  loadDetail,
  loadOperations,
  params,
  retryRun,
}: {
  cancelRun: (input: RunActionInput) => Promise<void>
  initialOperations: RepoOperations | null
  loadDetail: (input: RunActionInput) => Promise<RepoRunDetail>
  loadOperations: (input: RepoParams) => Promise<RepoOperations | null>
  params: RepoParams
  retryRun: (input: RunActionInput) => Promise<void>
}) {
  const [view, setView] = useState(() => ({
    actionError: null as { message: string; runId: string } | null,
    detail: null as RepoRunDetail | null,
    detailError: null as string | null,
    operations: initialOperations,
    pendingRunId: null as string | null,
    refreshError: null as string | null,
    selectedRunId: null as string | null,
  }))
  const detailGeneration = useRef(0)
  const mountedRef = useRef(false)
  const operationsGeneration = useRef(0)
  const operationsRef = useRef(initialOperations)
  const refreshInFlightRef = useRef<Promise<void> | null>(null)
  const selectedRunIdRef = useRef<string | null>(null)
  const {
    actionError,
    detail,
    detailError,
    operations,
    pendingRunId,
    refreshError,
    selectedRunId,
  } = view
  const canPoll = operations !== null
  const { owner, repo } = params

  const loadSelectedDetail = useCallback((runId: string) => {
    const generation = ++detailGeneration.current
    return loadDetail({ owner, repo, run_id: runId })
      .then((nextDetail) => {
        if (!mountedRef.current) return
        setView((current) =>
          generation === detailGeneration.current &&
          current.selectedRunId === runId
            ? { ...current, detail: nextDetail, detailError: null }
            : current,
        )
      })
      .catch((loadError: unknown) => {
        if (!mountedRef.current) return
        setView((current) =>
          generation === detailGeneration.current &&
          current.selectedRunId === runId
            ? { ...current, detailError: errorMessage(loadError) }
            : current,
        )
      })
  }, [loadDetail, owner, repo])

  const refresh = useCallback(() => {
    if (refreshInFlightRef.current) return refreshInFlightRef.current

    const detailRunId = selectedRunIdRef.current
    const previousOperations = operationsRef.current
    const refreshDetail = shouldRefreshSelectedRunDetail(
      previousOperations,
      detailRunId,
    )
    const operationsRequestGeneration = ++operationsGeneration.current
    const detailRequestGeneration = refreshDetail
      ? ++detailGeneration.current
      : null
    const request = Promise.allSettled([
      loadOperations({ owner, repo }),
      refreshDetail && detailRunId
        ? loadDetail({ owner, repo, run_id: detailRunId })
        : Promise.resolve(null),
    ]).then(([operationsResult, detailResult]) => {
      if (!mountedRef.current) return

      const operationsApply =
        operationsRequestGeneration === operationsGeneration.current
      const nextOperations =
        operationsApply && operationsResult.status === 'fulfilled'
          ? operationsResult.value
          : undefined
      const selectedRunRemoved =
        nextOperations !== undefined &&
        selectedRunIdRef.current === detailRunId &&
        selectedRunIsUnavailable(nextOperations, detailRunId)
      const refreshFinalDetail =
        nextOperations !== undefined &&
        selectedRunIdRef.current === detailRunId &&
        selectedRunBecameTerminal(
          previousOperations,
          nextOperations,
          detailRunId,
        )
      if (nextOperations !== undefined) {
        operationsRef.current = nextOperations
      }
      if (selectedRunRemoved) {
        selectedRunIdRef.current = null
        ++detailGeneration.current
      }

      setView((current) => {
        if (operationsApply && nextOperations === null) {
          return {
            actionError: null,
            detail: null,
            detailError: null,
            operations: null,
            pendingRunId: null,
            refreshError: null,
            selectedRunId: null,
          }
        }

        const detailApplies =
          detailRequestGeneration !== null &&
          detailRequestGeneration === detailGeneration.current &&
          current.selectedRunId === detailRunId
        if (!detailApplies && !operationsApply) return current

        const nextRefreshError = operationsApply
          ? operationsResult.status === 'rejected'
            ? errorMessage(operationsResult.reason)
            : null
          : current.refreshError
        const nextDetailError = detailApplies
          ? detailResult.status === 'rejected'
            ? errorMessage(detailResult.reason)
            : null
          : current.detailError

        return {
          ...current,
          detail:
            selectedRunRemoved
              ? null
              : detailApplies && detailResult.status === 'fulfilled'
              ? detailResult.value
              : current.detail,
          detailError: selectedRunRemoved ? null : nextDetailError,
          operations:
            operationsApply &&
            operationsResult.status === 'fulfilled' &&
            operationsResult.value
              ? operationsResult.value
              : current.operations,
          refreshError: nextRefreshError,
          selectedRunId: selectedRunRemoved ? null : current.selectedRunId,
        }
      })

      if (refreshFinalDetail && detailRunId) {
        return loadSelectedDetail(detailRunId)
      }
    }).finally(() => {
      if (refreshInFlightRef.current === request) {
        refreshInFlightRef.current = null
      }
    })
    refreshInFlightRef.current = request
    return request
  }, [loadDetail, loadOperations, loadSelectedDetail, owner, repo])

  useEffect(() => {
    mountedRef.current = true
    if (!canPoll) {
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
  }, [canPoll, refresh])

  if (!operations) {
    return (
      <>
        <RunsHeader />
        <div className="px-4 pb-12 sm:px-6 lg:px-8">
          <PageErrorAlert title="Runs unavailable">
            Sign in as the owner or a repository member to view runs and
            attached runners.
          </PageErrorAlert>
        </div>
      </>
    )
  }

  async function selectRun(runId: string) {
    if (selectedRunId === runId) {
      ++detailGeneration.current
      selectedRunIdRef.current = null
      setView((current) => ({
        ...current,
        detail: null,
        detailError: null,
        selectedRunId: null,
      }))
      return
    }
    selectedRunIdRef.current = runId
    setView((current) => ({
      ...current,
      detail: null,
      detailError: null,
      selectedRunId: runId,
    }))
    await loadSelectedDetail(runId)
  }

  async function runAction(
    runId: string,
    action: (input: RunActionInput) => Promise<void>,
  ) {
    ++operationsGeneration.current
    if (selectedRunIdRef.current === runId) {
      ++detailGeneration.current
    }
    setView((current) => ({
      ...current,
      actionError: current.actionError?.runId === runId
        ? null
        : current.actionError,
      pendingRunId: runId,
    }))
    try {
      await action({ owner, repo, run_id: runId })
        .then(() => refreshInFlightRef.current)
        .then(() => refresh())
    } catch (actionError) {
      if (!mountedRef.current) return
      setView((current) => ({
        ...current,
        actionError: {
          message: errorMessage(actionError),
          runId,
        },
      }))
    } finally {
      if (mountedRef.current) {
        setView((current) => current.pendingRunId === runId
          ? { ...current, pendingRunId: null }
          : current)
      }
    }
  }

  return (
    <>
      <RunsHeader runCount={operations.runs.length} />
      <div className="px-4 pb-12 sm:px-6 lg:px-8">
        {refreshError && (
          <PageErrorAlert title="Runs could not refresh">
            <div className="flex flex-wrap items-center gap-3">
              <span>{refreshError}</span>
              <Button onClick={() => void refresh()} size="sm" variant="secondary">
                Retry now
              </Button>
            </div>
          </PageErrorAlert>
        )}

        <RecentRuns
          actionError={actionError}
          detail={detail}
          detailError={detailError}
          onCancel={(runId) => void runAction(runId, cancelRun)}
          onDetailRetry={(runId) => void loadSelectedDetail(runId)}
          onRetry={(runId) => void runAction(runId, retryRun)}
          onSelect={(runId) => void selectRun(runId)}
          pendingRunId={pendingRunId}
          runs={operations.runs}
          selectedRunId={selectedRunId}
        />
        <Runners owner={owner} repo={repo} runners={operations.runners} />
      </div>
    </>
  )
}

function RecentRuns({
  actionError,
  detail,
  detailError,
  onCancel,
  onDetailRetry,
  onRetry,
  onSelect,
  pendingRunId,
  runs,
  selectedRunId,
}: {
  actionError: { message: string; runId: string } | null
  detail: RepoRunDetail | null
  detailError: string | null
  onCancel: (runId: string) => void
  onDetailRetry: (runId: string) => void
  onRetry: (runId: string) => void
  onSelect: (runId: string) => void
  pendingRunId: string | null
  runs: RepoOperations['runs']
  selectedRunId: string | null
}) {
  return (
    <section aria-labelledby="recent-runs-heading" className="pt-7 lg:pt-10">
      <div className="flex items-center gap-2">
        <TerminalSquare className="size-4 text-muted-foreground" />
        <h2 className="text-sm font-semibold" id="recent-runs-heading">
          Recent runs
        </h2>
        <span className="text-xs tabular-nums text-muted-foreground">
          {runs.length}
        </span>
      </div>
      <div className="mt-3 divide-y divide-border border-y border-border">
        {runs.length === 0 ? (
          <EmptyRunRow />
        ) : (
          runs.map((run) => (
            <RunRow
              actionError={
                actionError?.runId === run.id ? actionError.message : null
              }
              detail={selectedRunId === run.id ? detail : null}
              detailError={selectedRunId === run.id ? detailError : null}
              expanded={selectedRunId === run.id}
              key={run.id}
              onCancel={() => onCancel(run.id)}
              onDetailRetry={() => onDetailRetry(run.id)}
              onRetry={() => onRetry(run.id)}
              onSelect={() => onSelect(run.id)}
              pending={pendingRunId === run.id}
              run={run}
            />
          ))
        )}
      </div>
    </section>
  )
}

function Runners({
  owner,
  repo,
  runners,
}: {
  owner: string
  repo: string
  runners: RepoOperations['runners']
}) {
  return (
    <section aria-labelledby="runners-heading" className="pt-9">
      <div className="flex items-center gap-2">
        <Server className="size-4 text-muted-foreground" />
        <h2 className="text-sm font-semibold" id="runners-heading">
          Runners
        </h2>
        <span className="text-xs tabular-nums text-muted-foreground">
          {runners.length}
        </span>
      </div>
      <div className="mt-3 divide-y divide-border border-y border-border">
        {runners.length === 0 ? (
          <div className="px-2 py-5 text-sm text-muted-foreground">
            <p className="font-medium text-foreground">No runners attached</p>
            <p className="mt-1">
              Attach a machine with{' '}
              <code className="break-words">
                scope runner install --name &lt;name&gt; --repo {owner}/{repo}
              </code>
              .
            </p>
          </div>
        ) : (
          runners.map((runner) => (
            <div
              className="grid gap-2 px-2 py-4 text-sm sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-center sm:gap-6"
              key={runner.id}
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2 font-medium">
                  <StatusDot className="mt-0" state={runner.state} />
                  <span className="truncate">{runner.name}</span>
                </div>
                <p className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
                  {runner.id}
                </p>
              </div>
              <span className="text-xs text-muted-foreground">
                v{runner.version}
              </span>
              <div className="text-xs text-muted-foreground sm:text-right">
                <div className="capitalize text-foreground">{runner.state}</div>
                <div>
                  {runner.last_seen_at_unix
                    ? `Seen ${formatUnixTime(runner.last_seen_at_unix)}`
                    : 'Never connected'}
                </div>
              </div>
            </div>
          ))
        )}
      </div>
    </section>
  )
}

function RunsHeader({ runCount }: { runCount?: number }) {
  return (
    <WorkbenchHeader
      actions={(
        <code className="max-w-full overflow-x-auto whitespace-nowrap text-xs text-muted-foreground">
          scope run &lt;workflow&gt; --runner &lt;name&gt;
        </code>
      )}
      count={runCount === undefined
        ? undefined
        : `${runCount} recent ${runCount === 1 ? 'run' : 'runs'}`}
      description={(
        <>
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
        className="flex items-center gap-2 px-4 py-10 text-sm text-muted-foreground sm:px-6 lg:px-8"
      >
        <LoaderCircle className="size-4 animate-spin" />
        Loading runs
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

function RunRow({
  actionError,
  detail,
  detailError,
  expanded,
  onCancel,
  onDetailRetry,
  onRetry,
  onSelect,
  pending,
  run,
}: {
  actionError: string | null
  detail: RepoRunDetail | null
  detailError: string | null
  expanded: boolean
  onCancel: () => void
  onDetailRetry: () => void
  onRetry: () => void
  onSelect: () => void
  pending: boolean
  run: RepoRun
}) {
  const stateLabel = run.cancellation_requested ? 'canceling' : run.state
  const attemptLabel = run.attempt_number === 0
    ? null
    : run.state === 'queued'
      ? `last attempt ${run.attempt_number}`
      : `attempt ${run.attempt_number}`
  const metadata = [
    run.git_oid.slice(0, 12),
    run.desired_runner ?? 'any runner',
    attemptLabel,
  ].filter(Boolean).join(' · ')
  return (
    <div>
      <div className="flex min-w-0 flex-col gap-3 px-2 py-4 sm:flex-row sm:items-center">
        <button
          aria-controls={`run-detail-${run.id}`}
          aria-expanded={expanded}
          className="flex min-w-0 flex-1 items-start gap-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={onSelect}
          type="button"
        >
          {expanded ? (
            <ChevronDown className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronRight className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          )}
          <StatusDot state={run.state} />
          <span className="min-w-0">
            <span className="block truncate text-sm font-medium">{run.workflow_name}</span>
            <span className="mt-1 block truncate font-mono text-[11px] text-muted-foreground">
              {metadata}
            </span>
          </span>
        </button>
        <div className="flex items-center gap-3 pl-10 sm:pl-0">
          <div className="min-w-28 text-xs text-muted-foreground sm:text-right">
            <div className="capitalize text-foreground">{stateLabel}</div>
            <div>{formatUnixTime(run.updated_at_unix)}</div>
          </div>
          {run.can_cancel && (
            <Button
              aria-label={`Cancel ${run.workflow_name}`}
              disabled={pending}
              onClick={onCancel}
              size="sm"
              type="button"
              variant="ghost"
            >
              {pending ? <LoaderCircle className="animate-spin" /> : <Square />}
              Cancel
            </Button>
          )}
          {run.can_retry && (
            <Button
              aria-label={`Run ${run.workflow_name} again`}
              disabled={pending}
              onClick={onRetry}
              size="sm"
              type="button"
              variant="ghost"
            >
              {pending ? <LoaderCircle className="animate-spin" /> : <RotateCcw />}
              Run again
            </Button>
          )}
        </div>
      </div>
      {actionError && (
        <p className="px-10 pb-4 text-sm text-destructive" role="alert">
          {actionError}
        </p>
      )}
      {expanded && (
        <RunLogs
          detail={detail}
          error={detailError}
          id={`run-detail-${run.id}`}
          label={`${run.workflow_name} run details`}
          onRetry={onDetailRetry}
        />
      )}
    </div>
  )
}

function RunLogs({
  detail,
  error,
  id,
  label,
  onRetry,
}: {
  detail: RepoRunDetail | null
  error: string | null
  id: string
  label: string
  onRetry: () => void
}) {
  if (!detail && !error) {
    return (
      <section
        aria-label={label}
        className="flex items-center gap-2 border-t border-border bg-muted/20 px-10 py-5 text-sm text-muted-foreground"
        id={id}
      >
        <LoaderCircle className="size-4 animate-spin" />
        Loading logs
      </section>
    )
  }
  return (
    <section aria-label={label} id={id}>
      {error && (
        <div
          className="flex flex-wrap items-center gap-3 border-t border-border bg-destructive/5 px-10 py-3 text-sm text-destructive"
          role="alert"
        >
          <span>{error}</span>
          <Button onClick={onRetry} size="sm" type="button" variant="secondary">
            Retry logs
          </Button>
        </div>
      )}
      {detail && (
        <div className="border-t border-border bg-[#090b0e] text-[#eceae5]">
          <div className="flex items-center justify-between gap-3 border-b border-white/10 px-4 py-2 text-xs text-white/60">
            <span className="flex items-center gap-2">
              <TerminalSquare className="size-3.5" />
              Latest output
            </span>
            {detail.logs_truncated && <span>Earlier output omitted</span>}
          </div>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words px-4 py-4 font-mono text-xs leading-5">
            {detail.logs.length === 0
              ? <span className="text-white/50">No output yet.</span>
              : runLogText(detail)}
          </pre>
        </div>
      )}
    </section>
  )
}

function runLogText(detail: RepoRunDetail) {
  let previousAttempt: string | null = null
  return detail.logs.map((log) => {
    const separator = previousAttempt === log.attempt_id
      ? ''
      : `${previousAttempt ? '\n' : ''}── attempt …${log.attempt_id.slice(-8)} ──\n`
    previousAttempt = log.attempt_id
    return `${separator}${log.text}`
  }).join('')
}

function EmptyRunRow() {
  return (
    <div className="px-2 py-7">
      <p className="text-sm font-medium">No runs yet</p>
      <p className="mt-1 text-sm text-muted-foreground">
        Push to main with a matching trigger, or run a workflow manually from the CLI.
      </p>
    </div>
  )
}

function StatusDot({
  className,
  state,
}: {
  className?: string
  state: string
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'mt-1 size-2 shrink-0 rounded-full',
        ['online', 'running', 'succeeded'].includes(state) && 'bg-emerald-500',
        ['queued', 'leased'].includes(state) && 'bg-amber-500',
        ['failed', 'lost', 'offline'].includes(state) && 'bg-destructive',
        ['canceled', 'disabled'].includes(state) && 'bg-muted-foreground',
        className,
      )}
    />
  )
}

function formatUnixTime(value: number) {
  return DATE_FORMATTER.format(new Date(value * 1_000))
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Run operation failed.'
}
