import { AuthLayout } from '@/features/auth/auth-layout'
import {
  AuthLoadingState,
  AuthSurface,
} from '@/features/auth/auth-loading-state'
import { SignIn, useAuth } from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-in/$')({
  component: Page,
})

function Page() {
  const { isLoaded } = useAuth()
  return (
    <AuthLayout>
      <AuthSurface
        description="Continue to repositories, requests, and your CLI sessions."
        title="Sign in to Scope"
      >
        {isLoaded ? (
          <div className="scope-content-enter">
            <SignIn />
          </div>
        ) : <AuthLoadingState label="Loading sign in…" />}
      </AuthSurface>
    </AuthLayout>
  )
}
