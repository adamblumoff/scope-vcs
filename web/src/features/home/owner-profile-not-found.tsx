import { RouteErrorPage } from '@/components/route-error-page'

export function OwnerProfileNotFound() {
  return (
    <RouteErrorPage
      error={new Error('No user exists with that handle.')}
      fallbackMessage="User not found"
      title="Profile not found"
    />
  )
}
