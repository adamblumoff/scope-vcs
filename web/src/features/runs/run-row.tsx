import type { RepoParams, RepoRunHistoryPage } from '@/api/types'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { runDisplayState, runTriggerLabel } from './run-formatting'
import { runStatus } from './run-status'
import { RunDuration } from './run-duration'
import { RunStatusIcon } from './run-status-icon'
import { RunTimestamp } from './run-timestamp'
import {
  RUN_ROW_CLASS,
  RUN_ROW_DURATION_CLASS,
  RUN_ROW_PRIMARY_CLASS,
  RUN_ROW_TIMESTAMP_CLASS,
} from './run-row-layout'

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
        RUN_ROW_CLASS,
        'group outline-none transition-colors hover:bg-accent/50 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
        isRunning && 'bg-info-soft/40',
      )}
      params={{ ...params, runId: run.id }}
      to="/$owner/$repo/runs/$runId"
    >
      <RunStatusIcon state={state} />
      <span className={RUN_ROW_PRIMARY_CLASS}>
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
      <span className={`${RUN_ROW_DURATION_CLASS} text-right text-xs tabular-nums text-muted-foreground`}>
        <RunDuration end={run.completed_at_unix} start={run.created_at_unix} />
      </span>
      <span className={`${RUN_ROW_TIMESTAMP_CLASS} text-right text-xs tabular-nums text-muted-foreground`}>
        <RunTimestamp value={run.updated_at_unix} />
      </span>
    </Link>
  )
}
