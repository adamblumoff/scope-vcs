import type { RepoParams, RepoRunHistoryPage, RepoRunWorkflowList } from '@/api/types'
import { cn } from '@/lib/utils'
import { useNavigate } from '@tanstack/react-router'
import { runDisplayState } from './run-formatting'
import { type RunTone, runStatus } from './run-status'

export type RunStatusFilter = 'any' | 'failed' | 'running' | 'succeeded'

const STATUS_FILTER_TONE: Partial<Record<RunStatusFilter, RunTone>> = {
  failed: 'danger',
  running: 'running',
  succeeded: 'success',
}

const STATUS_FILTER_OPTIONS: { label: string; value: RunStatusFilter }[] = [
  { label: 'Any status', value: 'any' },
  { label: 'Running', value: 'running' },
  { label: 'Failed', value: 'failed' },
  { label: 'Succeeded', value: 'succeeded' },
]

const SELECT_CLASS = 'h-8 rounded-md border border-input bg-secondary px-2 text-sm text-foreground shadow-[var(--shadow-card)] outline-none transition-colors focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring'

export function runMatchesStatusFilter(
  run: RepoRunHistoryPage['runs'][number],
  filter: RunStatusFilter,
) {
  if (filter === 'any') return true
  const tone = runStatus(runDisplayState(run)).tone
  return tone === STATUS_FILTER_TONE[filter]
}

export function RunsFilterBar({
  onStatusFilterChange,
  params,
  selectedWorkflow,
  statusFilter,
  workflows,
}: {
  onStatusFilterChange: (filter: RunStatusFilter) => void
  params: RepoParams
  selectedWorkflow?: string
  statusFilter: RunStatusFilter
  workflows: RepoRunWorkflowList['workflows']
}) {
  const navigate = useNavigate()

  return (
    <div className="flex flex-wrap items-center gap-2">
      <select
        aria-label="Filter by workflow"
        className={cn(SELECT_CLASS, 'max-w-44')}
        onChange={(event) => {
          const value = event.target.value
          if (value === '') {
            void navigate({ params, to: '/$owner/$repo/runs' })
            return
          }
          void navigate({
            params: { ...params, workflow: value },
            to: '/$owner/$repo/runs/workflows/$workflow',
          })
        }}
        value={selectedWorkflow ?? ''}
      >
        <option value="">All workflows</option>
        {workflows.map((item) => (
          <option key={item.key} value={item.key}>
            {item.name}
          </option>
        ))}
      </select>
      <select
        aria-label="Filter by status"
        className={SELECT_CLASS}
        onChange={(event) => onStatusFilterChange(event.target.value as RunStatusFilter)}
        value={statusFilter}
      >
        {STATUS_FILTER_OPTIONS.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </div>
  )
}
