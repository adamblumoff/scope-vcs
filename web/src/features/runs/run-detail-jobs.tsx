import type { RepoRunJobDetail } from '@/api/types'
import { useMemo } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { StepLogState, StepSelection } from './repository-run-detail-controller'
import { attemptForJob } from './repository-run-detail-model'
import { RunDetailSteps } from './run-detail-steps'
import { RunDuration } from './run-duration'
import { runJobPanelId } from './run-job-ids'
import { RunJobGraph } from './run-job-graph'
import { orderJobsByDependency } from './run-job-graph-model'
import { RunStatusIcon } from './run-status-icon'

/** The Jobs section: a job strip (or dependency graph, behind a toggle) and
 * the steps of whichever job is selected. */
export function RunDetailJobs({
  attemptOverrides,
  jobs,
  onLogRetry,
  onSelectAttempt,
  onSelectJob,
  onSelectStep,
  onToggleGraph,
  selectedJobKey,
  selectedLogState,
  selection,
  showGraph,
}: {
  attemptOverrides: Readonly<Record<string, string>>
  jobs: readonly RepoRunJobDetail[]
  onLogRetry: () => void
  onSelectAttempt: (jobKey: string, attemptId: string) => void
  onSelectJob: (job: RepoRunJobDetail) => void
  onSelectStep: (jobKey: string, attemptId: string, stepIndex: number) => void
  onToggleGraph: () => void
  selectedJobKey: string | null
  selectedLogState: StepLogState
  selection: StepSelection | null
  showGraph: boolean
}) {
  const selectedJob = jobs.find(({ job }) => job.key === selectedJobKey) ?? null
  const orderedJobs = useMemo(() => orderJobsByDependency(jobs), [jobs])

  return (
    <section aria-labelledby="jobs-heading" className="pt-7">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sm font-semibold" id="jobs-heading">
          Jobs
        </h2>
        <div className="flex items-center gap-3">
          <p className="text-xs text-muted-foreground">{jobSummary(jobs)}</p>
          <Button
            aria-pressed={showGraph}
            onClick={onToggleGraph}
            size="sm"
            variant={showGraph ? 'secondary' : 'ghost'}
          >
            Graph
          </Button>
        </div>
      </div>
      <div className="mt-3">
        {showGraph ? (
          <RunJobGraph
            jobs={jobs}
            onSelectJob={onSelectJob}
            selectedJobKey={selectedJobKey}
          />
        ) : (
          <RunJobStrip
            jobs={orderedJobs}
            onSelectJob={onSelectJob}
            selectedJobKey={selectedJobKey}
          />
        )}
      </div>
      {selectedJob ? (
        <div
          className="mt-6 border-t border-border"
          id={runJobPanelId(selectedJob.job.key)}
        >
          <RunDetailSteps
            attempt={attemptForJob(selectedJob, attemptOverrides, selection)}
            jobDetail={selectedJob}
            onLogRetry={onLogRetry}
            onSelectAttempt={(attemptId) =>
              onSelectAttempt(selectedJob.job.key, attemptId)}
            onSelectStep={(attemptId, stepIndex) =>
              onSelectStep(selectedJob.job.key, attemptId, stepIndex)}
            selectedLogState={selectedLogState}
            selection={selection}
          />
        </div>
      ) : null}
    </section>
  )
}

function RunJobStrip({
  jobs,
  onSelectJob,
  selectedJobKey,
}: {
  jobs: readonly RepoRunJobDetail[]
  onSelectJob: (job: RepoRunJobDetail) => void
  selectedJobKey: string | null
}) {
  if (jobs.length === 0) {
    return (
      <p className="border-y border-border px-2 py-6 text-sm text-muted-foreground">
        This workflow has no jobs.
      </p>
    )
  }
  return (
    <div className="flex gap-2 overflow-x-auto border-y border-border py-3">
      {jobs.map((jobDetail) => {
        const { job } = jobDetail
        const selected = job.key === selectedJobKey
        return (
          <button
            aria-controls={runJobPanelId(job.key)}
            aria-pressed={selected}
            className={cn(
              'flex shrink-0 items-center gap-2 border bg-background px-3 py-2 text-left text-sm outline-none transition-colors hover:border-foreground/35 hover:bg-muted/20 focus-visible:ring-2 focus-visible:ring-ring',
              selected && 'border-foreground/50 ring-1 ring-foreground/10',
            )}
            key={job.key}
            onClick={() => onSelectJob(jobDetail)}
            type="button"
          >
            <RunStatusIcon state={job.state} />
            <span className="font-medium">{job.key}</span>
            <span className="text-xs text-muted-foreground">
              <RunDuration end={job.completed_at_unix} start={job.started_at_unix} />
            </span>
          </button>
        )
      })}
    </div>
  )
}

function jobSummary(jobs: readonly RepoRunJobDetail[]) {
  const counts = new Map<string, number>()
  for (const { job } of jobs) {
    counts.set(job.state, (counts.get(job.state) ?? 0) + 1)
  }
  return [...counts.entries()]
    .map(([state, count]) => `${count} ${state}`)
    .join(' · ') || 'No jobs'
}
