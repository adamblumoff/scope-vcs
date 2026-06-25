import {
  completeCliLoginForRequest,
} from '@/api/cli-login'
import { parseCompleteCliLoginInput } from '@/api/cli-login-input'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { SignInButton, useUser } from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { CheckCircle2, LoaderCircle, LogIn, ShieldCheck } from 'lucide-react'
import { type FormEvent, useState } from 'react'

const completeCliLogin = createServerFn({ method: 'POST' })
  .validator(parseCompleteCliLoginInput)
  .handler(({ data }) => completeCliLoginForRequest(data))

export const Route = createFileRoute('/cli-login')({
  component: CliLoginRoute,
})

function CliLoginRoute() {
  const { isLoaded, isSignedIn } = useUser()
  const [code, setCode] = useState('')
  const [state, setState] = useState<
    | { kind: 'idle' }
    | { kind: 'pending' }
    | { kind: 'complete' }
    | { kind: 'error'; message: string }
  >({ kind: 'idle' })

  async function authorizeCli(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setState({ kind: 'pending' })
    try {
      await completeCliLogin({ data: { code } })
      setCode('')
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

        {state.kind !== 'complete' && (
          <form
            className="mt-8 border-y border-border py-5"
            onSubmit={(event) => void authorizeCli(event)}
          >
            <div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm font-semibold leading-5">
                  <ShieldCheck className="size-4" />
                  <span>Terminal authorization</span>
                </div>
                <label className="mt-3 block text-xs font-medium leading-4 text-muted-foreground">
                  Code
                </label>
                <Input
                  autoCapitalize="characters"
                  autoComplete="one-time-code"
                  className="mt-1 max-w-[320px] font-mono uppercase"
                  disabled={state.kind === 'pending'}
                  inputMode="text"
                  onChange={(event) => setCode(event.target.value)}
                  placeholder="A1B2-C3D4-E5F6-A7B8"
                  value={code}
                />
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
                  disabled={state.kind === 'pending' || code.trim().length === 0}
                  size="sm"
                  type="submit"
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
          </form>
        )}
      </PageContent>
    </main>
  )
}
