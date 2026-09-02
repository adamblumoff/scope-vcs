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
  SignUp,
} from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-up/$')({
  component: Page,
})

function Page() {
  return (
    <AuthLayout>
      <AuthSurface
        description="Create an account for permissioned repository collaboration."
        title="Create your Scope account"
      >
        <ClerkLoading>
          <AuthLoadingState label="Loading sign up…" />
        </ClerkLoading>
        <ClerkFailed>
          <AuthFailureState title="Sign up unavailable" />
        </ClerkFailed>
        <ClerkLoaded>
          <div className="scope-content-enter">
            <SignUp />
          </div>
        </ClerkLoaded>
      </AuthSurface>
    </AuthLayout>
  )
}
