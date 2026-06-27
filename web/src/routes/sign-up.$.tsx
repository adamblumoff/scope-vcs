import { AuthLayout } from '@/features/auth/auth-layout'
import { SignUp } from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-up/$')({
  component: Page,
})

function Page() {
  return (
    <AuthLayout>
      <SignUp />
    </AuthLayout>
  )
}
