import type { RepoRunJobDetail } from '@/api/types'
import { cn } from '@/lib/utils'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useMemo } from 'react'
import { formatRunUnixTime } from './run-formatting'
import {
  JOB_GRAPH_NODE_HEIGHT,
  JOB_GRAPH_NODE_WIDTH,
  buildRunJobGraph,
} from './run-job-graph-model'
import { runJobPanelId } from './run-job-ids'
import { RunStatusDot } from './repository-runs-page'

export function RunJobGraph({
  expandedJobs,
  jobs,
  onToggleJob,
}: {
  expandedJobs: ReadonlySet<string>
  jobs: readonly RepoRunJobDetail[]
  onToggleJob: (job: RepoRunJobDetail) => void
}) {
  const layout = useMemo(() => buildRunJobGraph(jobs), [jobs])
  const jobsByKey = new Map(jobs.map((job) => [job.job.key, job]))

  if (jobs.length === 0) {
    return (
      <p className="border-y border-border px-2 py-6 text-sm text-muted-foreground">
        This workflow has no jobs.
      </p>
    )
  }

  return (
    <div
      aria-label="Job dependency graph"
      className="max-h-[min(70vh,42rem)] overflow-auto border-y border-border bg-muted/15 py-3"
    >
      <div
        className="relative"
        style={{ height: layout.height, minWidth: layout.width, width: layout.width }}
      >
        <svg
          aria-hidden="true"
          className="absolute inset-0 size-full overflow-visible"
          viewBox={`0 0 ${layout.width} ${layout.height}`}
        >
          <defs>
            <marker
              id="run-job-edge-arrow"
              markerHeight="6"
              markerWidth="6"
              orient="auto-start-reverse"
              refX="5"
              refY="3"
            >
              <path className="fill-muted-foreground/70" d="M 0 0 L 6 3 L 0 6 z" />
            </marker>
          </defs>
          {layout.edges.map((edge) => (
            <path
              className="fill-none stroke-border"
              d={edge.path}
              key={edge.key}
              markerEnd="url(#run-job-edge-arrow)"
              strokeWidth="2"
            />
          ))}
        </svg>
        {layout.nodes.map((node) => {
          const jobDetail = jobsByKey.get(node.key)
          if (!jobDetail) return null
          const { job, attempts } = jobDetail
          const expanded = expandedJobs.has(job.key)
          return (
            <button
              aria-controls={runJobPanelId(job.key)}
              aria-expanded={expanded}
              className={cn(
                'absolute flex flex-col justify-between border bg-background px-3 py-2.5 text-left shadow-sm outline-none transition-colors hover:border-foreground/35 hover:bg-muted/20 focus-visible:ring-2 focus-visible:ring-ring',
                expanded && 'border-foreground/50 ring-1 ring-foreground/10',
              )}
              key={job.key}
              onClick={() => onToggleJob(jobDetail)}
              style={{
                height: JOB_GRAPH_NODE_HEIGHT,
                left: node.x,
                top: node.y,
                width: JOB_GRAPH_NODE_WIDTH,
              }}
              type="button"
            >
              <span className="flex min-w-0 items-center gap-2">
                <RunStatusDot state={job.state} />
                <span className="min-w-0 flex-1 truncate text-sm font-semibold">{job.key}</span>
                {expanded
                  ? <ChevronDown className="size-3.5 text-muted-foreground" />
                  : <ChevronRight className="size-3.5 text-muted-foreground" />}
              </span>
              <span className="flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
                <span className="capitalize">{job.state}</span>
                <span>{attempts.length} {attempts.length === 1 ? 'attempt' : 'attempts'}</span>
              </span>
              <span className="truncate text-[10px] text-muted-foreground/80">
                {job.needs.length > 0
                  ? `After ${job.needs.join(', ')}`
                  : `Updated ${formatRunUnixTime(job.updated_at_unix)}`}
              </span>
            </button>
          )
        })}
      </div>
    </div>
  )
}
