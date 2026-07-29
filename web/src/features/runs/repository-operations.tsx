import type {
  RepoOperations,
  RepoParams,
  RepoRun,
  RepoRunDetail,
  RunActionInput,
} from '@/api/types'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  ChevronDown,
  ChevronRight,
  LoaderCircle,
  RotateCcw,
  Server,
  Square,
  TerminalSquare,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

const DATE_FORMATTER = new Intl.DateTimeFormat('en-US', {
  dateStyle: 'medium',
  timeZone: 'UTC',
  timeStyle: 'short',
})
const ACTIVE_RUN_STATES = new Set(['queued', 'leased', 'running'])

export function RepositoryOperations({
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
    detail: null as RepoRunDetail | null,
    error: null as string | null,
    operations: initialOperations,
    pendingRunId: null as string | null,
    selectedRunId: null as string | null,
  }))
  const detailGeneration = useRef(0)
  const operationsGeneration = useRef(0)
  const selectedActiveRunIdRef = useRef<string | null>(null)
  const selectedRunIdRef = useRef<string | null>(null)
  const { detail, error, operations, pendingRunId, selectedRunId } = view
  const canPoll = operations !== null
  const hasActiveRun = operations?.runs.some((run) =>
    ACTIVE_RUN_STATES.has(run.state),
  ) ?? false
  const selectedRunIsActive = operations?.runs.some(
    (run) => run.id === selectedRunId && ACTIVE_RUN_STATES.has(run.state),
  ) ?? false

  useEffect(() => {
    const needsFinalDetail =
      selectedRunId !== null &&
      selectedActiveRunIdRef.current === selectedRunId &&
      !selectedRunIsActive
    selectedActiveRunIdRef.current = selectedRunIsActive
      ? selectedRunId
      : null
    if (!canPoll) return
    let disposed = false

    const refresh = async (includeTerminalDetail = false) => {
      const refreshesDetail =
        selectedRunId !== null &&
        (selectedRunIsActive || includeTerminalDetail)
      const operationsRequestGeneration = ++operationsGeneration.current
      const detailRequestGeneration = refreshesDetail
        ? ++detailGeneration.current
        : null
      const [operationsResult, detailResult] = await Promise.allSettled([
        loadOperations(params),
        refreshesDetail
          ? loadDetail({ ...params, run_id: selectedRunId })
          : Promise.resolve(null),
      ])
      if (disposed) return

      setView((current) => {
        const detailApplies =
          detailRequestGeneration !== null &&
          detailRequestGeneration === detailGeneration.current &&
          current.selectedRunId === selectedRunId
        const operationsApply =
          operationsRequestGeneration === operationsGeneration.current
        if (!detailApplies && !operationsApply) return current

        const refreshError =
          operationsApply && operationsResult.status === 'rejected'
            ? errorMessage(operationsResult.reason)
            : detailApplies && detailResult.status === 'rejected'
              ? errorMessage(detailResult.reason)
              : detailApplies || current.selectedRunId === null
                ? null
                : current.error

        return {
          ...current,
          detail:
            detailApplies && detailResult.status === 'fulfilled'
              ? detailResult.value
              : current.detail,
          error: refreshError,
          operations:
            operationsApply &&
            operationsResult.status === 'fulfilled' &&
            operationsResult.value
              ? operationsResult.value
              : current.operations,
        }
      })
    }
    const refreshInterval = hasActiveRun ? 5_000 : 15_000
    const timer = window.setInterval(() => void refresh(), refreshInterval)
    const finalDetailTimer = needsFinalDetail
      ? window.setTimeout(() => void refresh(true), 0)
      : null
    return () => {
      disposed = true
      window.clearInterval(timer)
      if (finalDetailTimer !== null) window.clearTimeout(finalDetailTimer)
    }
  }, [
    canPoll,
    hasActiveRun,
    loadDetail,
    loadOperations,
    params,
    selectedRunIsActive,
    selectedRunId,
  ])

  if (!operations) return null

  async function selectRun(runId: string) {
    const generation = ++detailGeneration.current
    if (selectedRunId === runId) {
      selectedRunIdRef.current = null
      setView((current) => ({
        ...current,
        detail: null,
        selectedRunId: null,
      }))
      return
    }
    selectedRunIdRef.current = runId
    setView((current) => ({
      ...current,
      detail: null,
      error: null,
      selectedRunId: runId,
    }))
    try {
      const nextDetail = await loadDetail({ ...params, run_id: runId })
      setView((current) =>
        generation === detailGeneration.current &&
        current.selectedRunId === runId
          ? { ...current, detail: nextDetail }
          : current,
      )
    } catch (loadError) {
      setView((current) =>
        generation === detailGeneration.current &&
        current.selectedRunId === runId
          ? { ...current, error: errorMessage(loadError) }
          : current,
      )
    }
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
      error: null,
      pendingRunId: runId,
    }))
    try {
      await action({ ...params, run_id: runId })
      const refreshesSelectedRun = selectedRunIdRef.current === runId
      const operationsRequestGeneration = ++operationsGeneration.current
      const detailRequestGeneration = refreshesSelectedRun
        ? ++detailGeneration.current
        : null
      const [operationsResult, detailResult] = await Promise.allSettled([
        loadOperations(params),
        refreshesSelectedRun
          ? loadDetail({ ...params, run_id: runId })
          : Promise.resolve(null),
      ])
      setView((current) => {
        const detailApplies =
          detailRequestGeneration !== null &&
          detailRequestGeneration === detailGeneration.current &&
          current.selectedRunId === runId
        const operationsApply =
          operationsRequestGeneration === operationsGeneration.current
        if (!detailApplies && !operationsApply) {
          return current.pendingRunId === runId
            ? { ...current, pendingRunId: null }
            : current
        }
        const refreshError =
          operationsApply && operationsResult.status === 'rejected'
            ? errorMessage(operationsResult.reason)
            : detailApplies && detailResult.status === 'rejected'
              ? errorMessage(detailResult.reason)
              : detailApplies || current.selectedRunId === null
                ? null
                : current.error

        return {
          ...current,
          detail:
            detailApplies && detailResult.status === 'fulfilled'
              ? detailResult.value
              : current.detail,
          error: refreshError,
          operations:
            operationsApply &&
            operationsResult.status === 'fulfilled' &&
            operationsResult.value
              ? operationsResult.value
              : current.operations,
          pendingRunId: null,
        }
      })
    } catch (actionError) {
      setView((current) => ({
        ...current,
        error: errorMessage(actionError),
        pendingRunId: null,
      }))
    }
  }

  return (
    <section aria-labelledby="repository-operations-heading" className="border-t border-border">
      <div className="px-5 py-6 sm:px-8">
        <OperationsHeader />
        {error && (
          <p className="mt-4 text-sm text-destructive" role="alert">{error}</p>
        )}

        <div className="mt-5 divide-y divide-border border-y border-border">
          {operations.runs.length === 0 ? (
            <EmptyRunRow />
          ) : (
            operations.runs.map((run) => (
              <RunRow
                detail={selectedRunId === run.id ? detail : null}
                expanded={selectedRunId === run.id}
                key={run.id}
                onCancel={() => runAction(run.id, cancelRun)}
                onRetry={() => runAction(run.id, retryRun)}
                onSelect={() => selectRun(run.id)}
                pending={pendingRunId === run.id}
                run={run}
              />
            ))
          )}
        </div>

        <div className="mt-9 flex items-center gap-2">
          <Server className="size-4 text-muted-foreground" />
          <h3 className="text-sm font-semibold">Runners</h3>
          <span className="text-xs tabular-nums text-muted-foreground">
            {operations.runners.length}
          </span>
        </div>
        <div className="mt-3 divide-y divide-border border-y border-border">
          {operations.runners.length === 0 ? (
            <div className="px-2 py-5 text-sm text-muted-foreground">
              Attach a machine with <code>scope runner install --name &lt;name&gt; --repo {params.owner}/{params.repo}</code>.
            </div>
          ) : (
            operations.runners.map((runner) => (
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
                <span className="text-xs text-muted-foreground">v{runner.version}</span>
                <div className="text-xs text-muted-foreground sm:text-right">
                  <div className="capitalize text-foreground">{runner.state}</div>
                  <div>{runner.last_seen_at_unix ? `Seen ${formatUnixTime(runner.last_seen_at_unix)}` : 'Never connected'}</div>
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    </section>
  )
}

function OperationsHeader() {
  return (
    <>
      <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">
        Execute
      </p>
      <div className="mt-1 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold" id="repository-operations-heading">Runs</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Jobs from <code>.scope/runs</code> on your attached machines.
          </p>
        </div>
        <code className="text-xs text-muted-foreground">scope run &lt;workflow&gt; --runner &lt;name&gt;</code>
      </div>
    </>
  )
}

function RunRow({
  detail,
  expanded,
  onCancel,
  onRetry,
  onSelect,
  pending,
  run,
}: {
  detail: RepoRunDetail | null
  expanded: boolean
  onCancel: () => void
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
      {expanded && <RunLogs detail={detail} />}
    </div>
  )
}

function RunLogs({ detail }: { detail: RepoRunDetail | null }) {
  if (!detail) {
    return (
      <div className="flex items-center gap-2 border-t border-border bg-muted/20 px-10 py-5 text-sm text-muted-foreground">
        <LoaderCircle className="size-4 animate-spin" />
        Loading logs
      </div>
    )
  }
  return (
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
