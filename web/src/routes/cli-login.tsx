import {
  completeBrowserCliLoginForRequest,
  completeCliLoginForRequest,
} from '@/api/cli-login'
import {
  parseCompleteBrowserCliLoginInput,
  parseCompleteCliLoginInput,
} from '@/api/cli-login-input'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  CLI_CALLBACK_FALLBACK_DELAY_MS,
  parseCliCallbackHandoffUrl,
} from '@/lib/cli-callback-handoff'
import { SignInButton, useUser } from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { CheckCircle2, LoaderCircle, LogIn, ShieldCheck } from 'lucide-react'
import { type FormEvent, useEffect, useState } from 'react'

const completeCliLogin = createServerFn({ method: 'POST' })
  .validator(parseCompleteCliLoginInput)
  .handler(({ data }) => completeCliLoginForRequest(data))

const completeBrowserCliLogin = createServerFn({ method: 'POST' })
  .validator(parseCompleteBrowserCliLoginInput)
  .handler(({ data }) => completeBrowserCliLoginForRequest(data))

type CliLoginSearch = {
  request_id?: string
}

export const Route = createFileRoute('/cli-login')({
  validateSearch: (search: Record<string, unknown>): CliLoginSearch => ({
    request_id:
      typeof search.request_id === 'string' &&
      search.request_id.startsWith('cli_browser_')
        ? search.request_id
        : undefined,
  }),
  component: CliLoginRoute,
})

function CliLoginRoute() {
  const search = Route.useSearch()
  const { isLoaded, isSignedIn } = useUser()
  const [code, setCode] = useState('')
  const [state, setState] = useState<CliLoginRouteState>({ kind: 'idle' })
  const browserRequestId = search.request_id
  const callbackUrl =
    state.kind === 'callback-handoff' ? state.callbackUrl : null

  useEffect(() => {
    if (!callbackUrl) {
      return
    }

    const timeout = window.setTimeout(() => {
      setState((current) =>
        current.kind === 'callback-handoff' &&
        current.callbackUrl === callbackUrl
          ? { ...current, fallbackAvailable: true }
          : current,
      )
    }, CLI_CALLBACK_FALLBACK_DELAY_MS)

    return () => window.clearTimeout(timeout)
  }, [callbackUrl])

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

  async function authorizeBrowserCli() {
    if (!browserRequestId) {
      return
    }

    setState({ kind: 'pending' })
    try {
      const result = await completeBrowserCliLogin({
        data: { requestId: browserRequestId },
      })
      setState({
        kind: 'callback-handoff',
        callbackUrl: parseCliCallbackHandoffUrl(result.callback_url),
        fallbackAvailable: false,
      })
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
          description={
            browserRequestId
              ? 'Approve the terminal session that opened this browser.'
              : 'Authorize this terminal session to create and publish a repository.'
          }
          title="Authorize Scope CLI"
        />

        {state.kind === 'error' && (
          <PageErrorAlert title="CLI authorization failed">
            {state.message}
          </PageErrorAlert>
        )}

        {(state.kind === 'complete' || state.kind === 'callback-handoff') && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>CLI authorized</AlertTitle>
            <AlertDescription>
              <p>Return to your terminal.</p>
              {state.kind === 'callback-handoff' &&
                state.fallbackAvailable && (
                  <div className="mt-3 flex flex-col items-start gap-2">
                    <p>
                      If your terminal is still waiting, finish the local CLI
                      callback manually.
                    </p>
                    <Button
                      asChild
                      className="no-underline hover:no-underline"
                      size="sm"
                      variant="secondary"
                    >
                      <a href={state.callbackUrl}>Finish terminal login</a>
                    </Button>
                  </div>
                )}
            </AlertDescription>
          </Alert>
        )}

        {state.kind === 'callback-handoff' && (
          <iframe
            aria-hidden="true"
            className="sr-only"
            onError={() =>
              setState((current) =>
                current.kind === 'callback-handoff' &&
                current.callbackUrl === state.callbackUrl
                  ? { ...current, fallbackAvailable: true }
                  : current,
              )
            }
            onLoad={() =>
              setState((current) =>
                current.kind === 'callback-handoff' &&
                current.callbackUrl === state.callbackUrl
                  ? { kind: 'complete' }
                  : current,
              )
            }
            sandbox=""
            src={state.callbackUrl}
            title="Scope CLI local callback"
          />
        )}

        {!isTerminalLoginComplete(state) && browserRequestId && (
          <section className="mt-8 border-y border-border py-5">
            <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm font-semibold leading-5">
                  <ShieldCheck className="size-4" />
                  <span>Terminal authorization</span>
                </div>
                <p className="mt-1 max-w-[560px] text-sm leading-5 text-muted-foreground">
                  This approves a short-lived Scope CLI session on this machine.
                </p>
              </div>
              <CliLoginAction
                isLoaded={isLoaded}
                isSignedIn={isSignedIn}
                isPending={state.kind === 'pending'}
                onAuthorize={() => void authorizeBrowserCli()}
                pendingLabel="Authorizing"
              />
            </div>
          </section>
        )}

        {!isTerminalLoginComplete(state) && !browserRequestId && (
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
                <label
                  className="mt-3 block text-xs font-medium leading-4 text-muted-foreground"
                  htmlFor="cli-login-code"
                >
                  Code
                </label>
                <Input
                  autoCapitalize="characters"
                  autoComplete="one-time-code"
                  className="mt-1 max-w-[320px] font-mono uppercase"
                  disabled={state.kind === 'pending'}
                  id="cli-login-code"
                  inputMode="text"
                  onChange={(event) => setCode(event.target.value)}
                  placeholder="A1B2-C3D4-E5F6-A7B8"
                  value={code}
                />
              </div>
              <CliLoginAction
                disabled={code.trim().length === 0}
                isLoaded={isLoaded}
                isPending={state.kind === 'pending'}
                isSignedIn={isSignedIn}
                pendingLabel="Authorizing"
              />
            </div>
          </form>
        )}
      </PageContent>
    </main>
  )
}

type CliLoginRouteState =
  | { kind: 'idle' }
  | { kind: 'pending' }
  | {
      kind: 'callback-handoff'
      callbackUrl: string
      fallbackAvailable: boolean
    }
  | { kind: 'complete' }
  | { kind: 'error'; message: string }

function isTerminalLoginComplete(state: CliLoginRouteState) {
  return state.kind === 'complete' || state.kind === 'callback-handoff'
}

function CliLoginAction({
  disabled = false,
  isLoaded,
  isPending,
  isSignedIn,
  onAuthorize,
  pendingLabel,
}: {
  disabled?: boolean
  isLoaded: boolean
  isPending: boolean
  isSignedIn: boolean | undefined
  onAuthorize?: () => void
  pendingLabel: string
}) {
  if (!isLoaded) {
    return (
      <Button disabled size="sm" type="button">
        <LoaderCircle className="size-3.5 animate-spin" />
        <span>Loading</span>
      </Button>
    )
  }

  if (!isSignedIn) {
    return (
      <SignInButton mode="modal">
        <Button size="sm" type="button">
          <LogIn className="size-3.5" />
          <span>Sign in</span>
        </Button>
      </SignInButton>
    )
  }

  return (
    <Button
      disabled={isPending || disabled}
      onClick={onAuthorize}
      size="sm"
      type={onAuthorize ? 'button' : 'submit'}
    >
      {isPending ? (
        <LoaderCircle className="size-3.5 animate-spin" />
      ) : (
        <ShieldCheck className="size-3.5" />
      )}
      <span>{isPending ? pendingLabel : 'Authorize'}</span>
    </Button>
  )
}
