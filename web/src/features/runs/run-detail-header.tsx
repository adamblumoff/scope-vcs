import type { RepoRunDetail, RunActionInput } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { LoaderCircle, RotateCcw, Square } from 'lucide-react'
import { runDisplayState, runTriggerLabel } from './run-formatting'
import { RunDuration } from './run-duration'
import { RunStatusIcon } from './run-status-icon'
import { runStatus } from './run-status'
import { RunTimestamp } from './run-timestamp'

export function RunDetailHeader({
  detail,
  metadataError,
  onCancel,
  onRefresh,
  onRetry,
  params,
  pendingAction,
}: {
  detail: RepoRunDetail
  metadataError: string | null
  onCancel: () => void
  onRefresh: () => void
  onRetry: () => void
  params: RunActionInput
  pendingAction: 'cancel' | 'retry' | null
}) {
  const { run } = detail
  const state = runDisplayState(run)
  const shortOid = run.git_oid.slice(0, 7)

  return (
    <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
      <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Link
          className="hover:text-foreground"
          params={{ owner: params.owner, repo: params.repo }}
          to="/$owner/$repo/runs"
        >
          Runs
        </Link>
        <span aria-hidden="true">/</span>
        <span className="truncate">{run.workflow_name}</span>
        <code>#{shortOid}</code>
      </p>
      <div className="mt-2 flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <h1 className="flex flex-wrap items-center gap-3 text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[30px]">
            {run.workflow_name}
            <span className="flex items-center gap-2 text-sm font-medium text-muted-foreground">
              <RunStatusIcon state={state} />
              {runStatus(state).label}
            </span>
          </h1>
          <p className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
            <RunDuration end={run.completed_at_unix} start={run.created_at_unix} />
            <span aria-hidden="true">·</span>
            <span>{runTriggerLabel(run.trigger)}</span>
            <span aria-hidden="true">·</span>
            <RunTimestamp value={run.updated_at_unix} />
          </p>
        </div>
        {/* Both controls keep their slot so the cluster never shifts when the
            run becomes cancellable or retryable. */}
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <Button
            disabled={!run.can_cancel || pendingAction !== null}
            onClick={onCancel}
            variant="secondary"
          >
            {pendingAction === 'cancel'
              ? <LoaderCircle className="animate-spin" />
              : <Square />}
            Cancel
          </Button>
          <Button
            disabled={!run.can_retry || pendingAction !== null}
            onClick={onRetry}
            variant="secondary"
          >
            {pendingAction === 'retry'
              ? <LoaderCircle className="animate-spin" />
              : <RotateCcw />}
            Run again
          </Button>
        </div>
      </div>
      {metadataError ? (
        <p
          className="mt-3 flex flex-wrap items-center gap-2 text-xs text-muted-foreground"
          role="status"
        >
          <span>Live updates paused. {metadataError}</span>
          <button
            className="underline underline-offset-2 hover:text-foreground"
            onClick={onRefresh}
            type="button"
          >
            Retry now
          </button>
        </p>
      ) : null}
    </header>
  )
}
