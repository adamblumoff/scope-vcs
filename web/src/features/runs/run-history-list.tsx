import type { RepoParams, RepoRunHistoryPage } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowRight, LoaderCircle, TerminalSquare } from 'lucide-react'
import { formatRunRunnerSelection, formatRunUnixTime } from './run-formatting'
import { RunStatusDot } from './run-status-dot'

export function RunHistoryList({
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
          <div className="px-2 py-7">
            <p className="text-sm font-medium">
              {selectedWorkflowName ? `No ${selectedWorkflowName} runs yet` : 'No runs yet'}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              Push to main with a matching trigger, or run a workflow manually from the CLI.
            </p>
          </div>
        ) : runs.map((run) => {
          const state = run.cancellation_requested ? 'canceling' : run.state
          return (
            <Link
              className="group flex min-h-16 min-w-0 items-center gap-3 px-2 py-4 outline-none hover:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              key={run.id}
              params={{ ...params, runId: run.id }}
              to="/$owner/$repo/runs/$runId"
            >
              <RunStatusDot state={run.state} />
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
        })}
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
