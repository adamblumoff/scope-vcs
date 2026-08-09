import type { RepoParams, RepoRunWorkflowList } from '@/api/types'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { GitBranch } from 'lucide-react'

export function WorkflowNavigation({
  params,
  selectedWorkflow,
  workflows,
}: {
  params: RepoParams
  selectedWorkflow?: string
  workflows: RepoRunWorkflowList['workflows']
}) {
  const baseClass = 'flex shrink-0 items-center gap-2.5 whitespace-nowrap rounded-md px-2.5 py-2 text-sm outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring lg:w-full'
  return (
    <nav
      aria-label="Run workflows"
      className="flex min-w-0 gap-1 overflow-x-auto border-b border-border px-3 py-3 lg:block lg:min-h-[32rem] lg:overflow-visible lg:border-b-0 lg:border-r lg:px-3 lg:py-6"
    >
      <p className="hidden px-2.5 pb-2 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground lg:block">
        Workflows
      </p>
      <Link
        activeOptions={{ exact: true }}
        className={cn(baseClass, !selectedWorkflow && 'bg-muted font-medium text-foreground')}
        params={params}
        to="/$owner/$repo/runs"
      >
        <GitBranch className="size-3.5" />
        All workflows
      </Link>
      {workflows.map((item) => (
        <Link
          className={cn(
            baseClass,
            selectedWorkflow === item.key && 'bg-muted font-medium text-foreground',
          )}
          key={item.key}
          params={{ ...params, workflow: item.key }}
          to="/$owner/$repo/runs/workflows/$workflow"
        >
          <span className="size-2 rounded-full border-2 border-current" />
          <span className="max-w-44 truncate">{item.name}</span>
        </Link>
      ))}
    </nav>
  )
}
