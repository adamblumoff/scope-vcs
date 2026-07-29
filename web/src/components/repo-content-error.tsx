import { RouteErrorContent } from '@/components/route-error-page'

export function RepoContentError({ error }: { error: unknown }) {
  return (
    <RouteErrorContent
      error={error}
      fallbackMessage="Unexpected repository content error"
      title="Repository content unavailable"
    />
  )
}
