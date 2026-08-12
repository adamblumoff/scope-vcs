import { LoaderCircle } from 'lucide-react'
import { WorkbenchPane } from '@/components/page-header'
import { RunsHeader } from './runs-header'

export function RunsPagePending() {
  return (
    <WorkbenchPane>
      <RunsHeader />
      <output
        aria-busy="true"
        className="flex items-center gap-2 border-t border-border px-4 py-10 text-sm text-muted-foreground sm:px-6 lg:px-8"
      >
        <LoaderCircle className="size-4 animate-spin" />
        Loading workflows and runs
      </output>
    </WorkbenchPane>
  )
}
