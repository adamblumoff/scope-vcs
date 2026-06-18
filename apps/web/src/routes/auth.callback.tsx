import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { createScopeShooAuth } from '@/lib/auth'
import { createFileRoute } from '@tanstack/react-router'
import { AlertCircle, LoaderCircle, LogIn } from 'lucide-react'
import { useEffect, useState } from 'react'

export const Route = createFileRoute('/auth/callback')({
  component: AuthCallback,
})

function AuthCallback() {
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false

    async function finishSignIn() {
      try {
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

        if (!cancelled) {
          window.location.replace('/')
        }
      } catch (authError) {
        if (!cancelled) {
          setError(
            authError instanceof Error ? authError.message : 'Sign in failed',
          )
        }
      }
    }

    void finishSignIn()

    return () => {
      cancelled = true
    }
  }, [])

  async function restartSignIn() {
    setError(null)
    await createScopeShooAuth().startSignIn({ requestPii: true })
  }

  return (
    <main className="grid min-h-screen place-items-center bg-background px-4 text-foreground">
      {error ? (
        <div className="w-full max-w-[520px]">
          <Alert variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Sign in failed</AlertTitle>
            <AlertDescription className="space-y-4">
              <p>{error}</p>
              <Button onClick={() => void restartSignIn()} size="sm">
                <LogIn className="size-3.5" />
                <span>Try again</span>
              </Button>
            </AlertDescription>
          </Alert>
        </div>
      ) : (
        <LoaderCircle
          aria-label="Finishing sign in"
          className="size-5 animate-spin text-muted-foreground"
        />
      )}
    </main>
  )
}
