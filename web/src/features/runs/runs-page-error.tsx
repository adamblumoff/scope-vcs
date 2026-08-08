import { RouteErrorContent } from '@/components/route-error-page'
import { RunsHeader } from './runs-header'

export function RunsPageError({ error }: { error: unknown }) {
  return (
    <>
      <RunsHeader />
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected runs error"
        title="Runs unavailable"
      />
    </>
  )
}
