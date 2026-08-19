import {
  createCliExchangeGrantForRequest,
  listCliSessionsForRequest,
  revokeCliSessionForRequest,
} from '@/api/cli-login'
import { parseRevokeCliSessionInput } from '@/api/cli-login-input'
import type { CliExchangeGrant } from '@/api/types'
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { AccountPagePending } from '@/features/account/account-page-pending'
import { CliSessionList } from '@/features/account/cli-session-list'
import { UserButton } from '@clerk/tanstack-react-start'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { KeyRound, LoaderCircle, Monitor, Plus } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

const requireAccountAuth = createServerFn({ method: 'GET' }).handler(async () => {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const { isAuthenticated } = await auth()
  if (!isAuthenticated) {
    throw redirect({ params: { _splat: '' }, to: '/sign-in/$' })
  }
})

const loadCliSessions = createServerFn({ method: 'GET' }).handler(
  listCliSessionsForRequest,
)

const createCliExchangeGrant = createServerFn({ method: 'POST' }).handler(
  createCliExchangeGrantForRequest,
)

const revokeCliSession = createServerFn({ method: 'POST' })
  .validator(parseRevokeCliSessionInput)
  .handler(({ data }) => revokeCliSessionForRequest(data))

const UNIX_TIME_FORMATTER = new Intl.DateTimeFormat('en-US', {
  dateStyle: 'medium',
  timeStyle: 'short',
})

export const Route = createFileRoute('/account')({
  beforeLoad: () => requireAccountAuth(),
  loader: () => loadCliSessions(),
  pendingComponent: AccountPagePending,
  component: AccountRoute,
})

function AccountRoute() {
  const loaded = Route.useLoaderData()
  const [grant, setGrant] = useState<CliExchangeGrant | null>(null)
  const [sessions, setSessions] = useState(() => loaded.sessions)
  const [pending, setPending] = useState<'grant' | string | null>(null)
  const [error, setError] = useState<string | null>(null)

  async function createGrant() {
    setPending('grant')
    setError(null)
    try {
      setGrant(await createCliExchangeGrant())
      toast.success('Login command created')
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Could not create login command')
    } finally {
      setPending(null)
    }
  }

  async function revokeSession(sessionId: string) {
    setPending(sessionId)
    setError(null)
    try {
      await revokeCliSession({ data: { sessionId } })
      setSessions((current) => current.filter((session) => session.id !== sessionId))
      toast.success('CLI session revoked')
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Could not revoke CLI session')
    } finally {
      setPending(null)
    }
  }

  return (
    <AppShell
      header={() => (
        <ApplicationTopbar contextLabel="Account">
          <UserButton />
        </ApplicationTopbar>
      )}
    >
      <PageContent>
        <PageHeader
          description="Manage Scope CLI access for this account."
          title="Account"
        />

        {error && (
          <PageErrorAlert title="CLI session update failed">
            {error}
          </PageErrorAlert>
        )}

        <SectionRows>
          <SectionRow
            description="Create a short-lived command for agents, remote shells, or another terminal."
            icon={<KeyRound className="size-4" />}
            title="One-time CLI login"
          >
            <div className="space-y-3">
              <Button
                disabled={pending === 'grant'}
                onClick={() => void createGrant()}
                size="sm"
                type="button"
              >
                {pending === 'grant' ? (
                  <LoaderCircle className="size-3.5 animate-spin" />
                ) : (
                  <Plus className="size-3.5" />
                )}
                <span>{grant ? 'Create another' : 'Create command'}</span>
              </Button>
              {grant && (
                <div className="space-y-2">
                  <CopyableCodeBlock value={`scope login --exchange ${grant.exchange_token}`} />
                  <p className="text-xs leading-4 text-muted-foreground">
                    Expires {formatUnixTime(grant.expires_at_unix)}.
                  </p>
                </div>
              )}
            </div>
          </SectionRow>

          <SectionRow
            description="Active sessions created by scope login or scope init."
            icon={<Monitor className="size-4" />}
            title="CLI sessions"
          >
            <CliSessionList
              formatTime={formatUnixTime}
              pending={pending}
              revokeSession={(sessionId) => void revokeSession(sessionId)}
              sessions={sessions}
            />
          </SectionRow>
        </SectionRows>
      </PageContent>
    </AppShell>
  )
}

function formatUnixTime(value: number) {
  return UNIX_TIME_FORMATTER.format(new Date(value * 1000))
}
