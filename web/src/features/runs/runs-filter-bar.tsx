import type { RepoParams, RepoRunWorkflowList } from '@/api/types'
import { cn } from '@/lib/utils'
import { useNavigate } from '@tanstack/react-router'
import {
  RUN_STATUS_FILTER_OPTIONS,
  type RunStatusFilter,
} from './runs-filter-model'

const SELECT_CLASS = 'h-8 rounded-md border border-input bg-secondary px-2 text-sm text-foreground shadow-[var(--shadow-card)] outline-none transition-colors focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring'

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
        {RUN_STATUS_FILTER_OPTIONS.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </div>
  )
}
