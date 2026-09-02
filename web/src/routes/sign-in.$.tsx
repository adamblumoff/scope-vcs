import { AuthLayout } from '@/features/auth/auth-layout'
import { AuthFailureState } from '@/features/auth/auth-failure-state'
import {
  AuthLoadingState,
  AuthSurface,
} from '@/features/auth/auth-loading-state'
import {
  ClerkFailed,
  ClerkLoaded,
  ClerkLoading,
  SignIn,
} from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-in/$')({
  component: Page,
})

function Page() {
  return (
    <AuthLayout>
      <AuthSurface
        description="Continue to repositories, requests, and your CLI sessions."
        title="Sign in to Scope"
      >
        <ClerkLoading>
          <AuthLoadingState label="Loading sign in…" />
        </ClerkLoading>
        <ClerkFailed>
          <AuthFailureState title="Sign in unavailable" />
        </ClerkFailed>
        <ClerkLoaded>
          <div className="scope-content-enter">
            <SignIn />
          </div>
        </ClerkLoaded>
      </AuthSurface>
    </AuthLayout>
  )
}
