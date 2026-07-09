import { AuthLayout } from '@/features/auth/auth-layout'
import {
  AuthLoadingState,
  AuthSurface,
} from '@/features/auth/auth-loading-state'
import { SignUp, useAuth } from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-up/$')({
  component: Page,
})

function Page() {
  const { isLoaded } = useAuth()
  return (
    <AuthLayout>
      <AuthSurface
        description="Create an account for permissioned repository collaboration."
        title="Create your Scope account"
      >
        {isLoaded ? <SignUp /> : <AuthLoadingState label="Loading sign up…" />}
      </AuthSurface>
    </AuthLayout>
  )
}
