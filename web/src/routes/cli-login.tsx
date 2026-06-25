import {
  completeCliLoginForRequest,
} from '@/api/cli-login'
import { parseCompleteCliLoginInput } from '@/api/cli-login-input'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { SignInButton, useUser } from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { CheckCircle2, LoaderCircle, LogIn, ShieldCheck } from 'lucide-react'
import { useState } from 'react'

const completeCliLogin = createServerFn({ method: 'POST' })
  .validator(parseCompleteCliLoginInput)
  .handler(({ data }) => completeCliLoginForRequest(data))

export const Route = createFileRoute('/cli-login')({
  validateSearch: (search) => ({
    code: typeof search.code === 'string' ? search.code : '',
  }),
  component: CliLoginRoute,
})

function CliLoginRoute() {
  const { code } = Route.useSearch()
  const { isLoaded, isSignedIn } = useUser()
  const [state, setState] = useState<
    | { kind: 'idle' }
    | { kind: 'pending' }
    | { kind: 'complete' }
    | { kind: 'error'; message: string }
  >({ kind: 'idle' })

  async function authorizeCli() {
    setState({ kind: 'pending' })
    try {
      await completeCliLogin({ data: { code } })
      setState({ kind: 'complete' })
    } catch (error) {
      setState({
        kind: 'error',
        message: error instanceof Error ? error.message : 'CLI authorization failed',
      })
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader subtitle="CLI login" />
      <PageContent>
        <PageHeader
          description="Authorize this terminal session to create and publish a repository."
          title="Authorize Scope CLI"
        />

        {!code && (
          <PageErrorAlert title="Missing login code">
            Start again from the Scope CLI.
          </PageErrorAlert>
        )}

        {state.kind === 'error' && (
          <PageErrorAlert title="CLI authorization failed">
            {state.message}
          </PageErrorAlert>
        )}

        {state.kind === 'complete' && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>CLI authorized</AlertTitle>
            <AlertDescription>Return to your terminal.</AlertDescription>
          </Alert>
        )}

        {code && state.kind !== 'complete' && (
          <div className="mt-8 border-y border-border py-5">
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm font-semibold leading-5">
                  <ShieldCheck className="size-4" />
                  <span>Terminal authorization</span>
                </div>
                <p className="mt-1 text-sm leading-5 text-muted-foreground">
                  Code <span className="font-mono text-foreground">{code}</span>
                </p>
              </div>
              {!isLoaded && (
                <Button disabled size="sm" type="button">
                  <LoaderCircle className="size-3.5 animate-spin" />
                  <span>Loading</span>
                </Button>
              )}
              {isLoaded && !isSignedIn && (
                <SignInButton mode="modal">
                  <Button size="sm" type="button">
                    <LogIn className="size-3.5" />
                    <span>Sign in</span>
                  </Button>
                </SignInButton>
              )}
              {isLoaded && isSignedIn && (
                <Button
                  disabled={state.kind === 'pending'}
                  onClick={() => void authorizeCli()}
                  size="sm"
                  type="button"
                >
                  {state.kind === 'pending' ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : (
                    <ShieldCheck className="size-3.5" />
                  )}
                  <span>
                    {state.kind === 'pending' ? 'Authorizing' : 'Authorize'}
                  </span>
                </Button>
              )}
            </div>
          </div>
        )}
      </PageContent>
    </main>
  )
}
