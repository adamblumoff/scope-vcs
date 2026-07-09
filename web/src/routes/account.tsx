import {
  createCliExchangeGrantForRequest,
  listCliSessionsForRequest,
  revokeCliSessionForRequest,
} from '@/api/cli-login'
import { parseRevokeCliSessionInput } from '@/api/cli-login-input'
import type { CliExchangeGrant, CliSession } from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { AppShell } from '@/components/app-shell'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { DestructiveActionDialog } from '@/components/destructive-action-dialog'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { UserButton } from '@clerk/tanstack-react-start'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { KeyRound, LoaderCircle, Monitor, Plus, Trash2 } from 'lucide-react'
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
      header={() => <AppHeader action={() => <UserButton />} subtitle="Account" />}
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

function CliSessionList({
  pending,
  revokeSession,
  sessions,
}: {
  pending: string | null
  revokeSession: (sessionId: string) => void
  sessions: CliSession[]
}) {
  const [confirmSession, setConfirmSession] = useState<CliSession | null>(null)

  if (sessions.length === 0) {
    return <p className="text-sm leading-5 text-muted-foreground">No active CLI sessions.</p>
  }

  return (
    <>
      <ul className="divide-y divide-border border-y border-border">
        {sessions.map((session) => (
          <li
            className="flex flex-col gap-3 py-3 sm:flex-row sm:items-center sm:justify-between"
            key={session.id}
          >
            <div className="min-w-0">
              <div className="truncate text-sm font-medium leading-5">
                {session.label}
              </div>
              <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs leading-4 text-muted-foreground">
                <span>Created {formatUnixTime(session.created_at_unix)}</span>
                {session.last_used_at_unix && (
                  <span>Used {formatUnixTime(session.last_used_at_unix)}</span>
                )}
                <span>Expires {formatUnixTime(session.expires_at_unix)}</span>
              </div>
            </div>
            <Button
              aria-label={`Revoke ${session.label}`}
              disabled={pending === session.id}
              onClick={() => setConfirmSession(session)}
              size="icon-sm"
              title={`Revoke ${session.label}`}
              type="button"
              variant="destructive"
            >
              {pending === session.id ? (
                <LoaderCircle className="size-3.5 animate-spin" />
              ) : (
                <Trash2 className="size-3.5" />
              )}
            </Button>
          </li>
        ))}
      </ul>
      <DestructiveActionDialog
        confirmLabel="Revoke session"
        description="This CLI session will lose access immediately."
        onConfirm={() => {
          if (confirmSession) {
            revokeSession(confirmSession.id)
            setConfirmSession(null)
          }
        }}
        onOpenChange={(open) => {
          if (!open && !pending) setConfirmSession(null)
        }}
        open={Boolean(confirmSession)}
        pending={Boolean(confirmSession && pending === confirmSession.id)}
        subject={confirmSession?.label ?? ''}
        title="Revoke CLI session?"
      />
    </>
  )
}

function formatUnixTime(value: number) {
  return UNIX_TIME_FORMATTER.format(new Date(value * 1000))
}
