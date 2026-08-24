import { RouteErrorContent } from '@/components/route-error-page'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'

export function RunsPageError({ error }: { error: unknown }) {
  return (
    <WorkbenchPane>
      <WorkbenchBar title="Runs" />
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected runs error"
        title="Runs unavailable"
      />
    </WorkbenchPane>
  )
}
