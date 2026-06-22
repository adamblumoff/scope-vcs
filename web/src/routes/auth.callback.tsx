import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { createScopeShooAuth } from '@/lib/auth'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { AlertCircle, LoaderCircle, LogIn } from 'lucide-react'

export const Route = createFileRoute('/auth/callback')({
  ssr: false,
  loader: finishSignIn,
  errorComponent: AuthCallbackError,
  component: AuthCallback,
})

async function finishSignIn() {
  const token = await createScopeShooAuth().finishSignIn()
  if (!token) {
    throw new Error('No Shoo callback was found.')
  }

  const response = await fetch('/auth/session', {
    body: JSON.stringify({
      expiresIn: token.expires_in,
      idToken: token.id_token,
    }),
    headers: {
      'content-type': 'application/json',
    },
    method: 'POST',
  })

  if (!response.ok) {
    const payload = await response.json().catch(() => null)
    throw new Error(payload?.error ?? `session failed: ${response.status}`)
  }

  createScopeShooAuth().clearIdentity()
  throw redirect({ to: '/' })
}

function AuthCallback() {
  return (
    <main className="grid min-h-screen place-items-center bg-background px-4 text-foreground">
      <LoaderCircle
        aria-label="Finishing sign in"
        className="size-5 animate-spin text-muted-foreground"
      />
    </main>
  )
}

async function restartSignIn() {
  await createScopeShooAuth().startSignIn({ requestPii: true })
}

function AuthCallbackError({ error }: { error: unknown }) {
  const message = error instanceof Error ? error.message : 'Sign in failed'

  return (
    <main className="grid min-h-screen place-items-center bg-background px-4 text-foreground">
      <div className="w-full max-w-[520px]">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Sign in failed</AlertTitle>
          <AlertDescription className="space-y-4">
            <p>{message}</p>
            <Button onClick={() => void restartSignIn()} size="sm">
              <LogIn className="size-3.5" />
              <span>Try again</span>
            </Button>
          </AlertDescription>
        </Alert>
      </div>
    </main>
  )
}
