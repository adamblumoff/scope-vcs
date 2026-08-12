import { RouteErrorContent } from '@/components/route-error-page'
import { WorkbenchPane } from '@/components/page-header'
import { RunsHeader } from './runs-header'

export function RunsPageError({ error }: { error: unknown }) {
  return (
    <WorkbenchPane>
      <RunsHeader />
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected runs error"
        title="Runs unavailable"
      />
    </WorkbenchPane>
  )
}
