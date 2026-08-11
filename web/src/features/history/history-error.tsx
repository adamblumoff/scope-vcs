import { RouteErrorContent } from '@/components/route-error-page'

export function HistoryError({ error }: { error: unknown }) {
  return (
    <RouteErrorContent
      error={error}
      fallbackMessage="Unexpected history error"
      title="History unavailable"
    />
  )
}
