import { RouteErrorPage } from '@/components/route-error-page'

export function OwnerProfileError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected profile error"
      title="Profile unavailable"
    />
  )
}
