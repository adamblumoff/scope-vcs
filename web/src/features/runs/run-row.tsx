import type { RepoParams, RepoRunHistoryPage } from '@/api/types'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { runDisplayState, runTriggerLabel } from './run-formatting'
import { runStatus } from './run-status'
import { RunDuration } from './run-duration'
import { RunStatusIcon } from './run-status-icon'
import { RunTimestamp } from './run-timestamp'

export function RunRow({
  params,
  run,
}: {
  params: RepoParams
  run: RepoRunHistoryPage['runs'][number]
}) {
  const state = runDisplayState(run)
  const isRunning = runStatus(state).tone === 'running'

  return (
    <Link
      className={cn(
        'group flex min-w-0 items-center gap-3 px-3 py-3 outline-none transition-colors hover:bg-accent/50 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
        isRunning && 'bg-info-soft/40',
      )}
      params={{ ...params, runId: run.id }}
      to="/$owner/$repo/runs/$runId"
    >
      <RunStatusIcon state={state} />
      <span className="min-w-0 flex-1 items-baseline gap-2 truncate sm:flex">
        <span className="truncate text-sm font-medium">
          {run.workflow_name}
        </span>
        <span className="truncate font-mono text-xs text-muted-foreground">
          #{run.git_oid.slice(0, 7)}
          <span className="text-muted-foreground/70">
            {' '}· {runTriggerLabel(run.trigger)}
          </span>
        </span>
      </span>
      <span className="w-16 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
        <RunDuration end={run.completed_at_unix} start={run.created_at_unix} />
      </span>
      <span className="w-24 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
        <RunTimestamp value={run.updated_at_unix} />
      </span>
    </Link>
  )
}
