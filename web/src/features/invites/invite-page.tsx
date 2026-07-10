import type {
  AcceptRepoInviteResponse,
  RepoInviteLookup,
  RepoInviteTokenInput,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link, useNavigate } from '@tanstack/react-router'
import { useAuth } from '@clerk/tanstack-react-start'
import { Check, LoaderCircle } from 'lucide-react'
import { useState } from 'react'

export function InvitePage({
  acceptInvite,
  invite,
  token,
}: {
  acceptInvite: (
    input: RepoInviteTokenInput,
  ) => Promise<AcceptRepoInviteResponse>
  invite: RepoInviteLookup
  token: string
}) {
  const { isLoaded, isSignedIn } = useAuth()
  const navigate = useNavigate()
  const [acceptError, setAcceptError] = useState<string | null>(null)
  const [accepting, setAccepting] = useState(false)
  const returnPath = `/invites/${encodeURIComponent(token)}`

  async function onAccept(input: RepoInviteTokenInput) {
    if (!isSignedIn || accepting) {
      return
    }

    setAcceptError(null)
    setAccepting(true)
    try {
      const accepted = await acceptInvite(input)
      await navigate({
        params: {
          owner: accepted.repo.owner_handle,
          repo: accepted.repo.name,
        },
        to: '/repos/$owner/$repo',
      })
    } catch (error) {
      setAcceptError(error instanceof Error ? error.message : 'invite failed')
    } finally {
      setAccepting(false)
    }
  }

  return (
    <AppShell header={() => <AppHeader />}>
      <PageContent>
        <PageHeader
          badges={() => (
            <>
              <Badge variant="info">Invite</Badge>
              <Badge variant="neutral">{invite.invited_email}</Badge>
            </>
          )}
          description={() => (
            <span className="font-mono text-muted-foreground">
              {invite.repo_id}
            </span>
          )}
          title="Repository invite"
        />

        <div className="mt-6 divide-y divide-border">
          <section className="grid gap-4 py-5 md:grid-cols-[220px_minmax(0,1fr)]">
            <div>
              <div className="text-sm font-semibold leading-5">Access</div>
              <p className="mt-1 text-sm leading-5 text-muted-foreground">
                Accepting this invite makes you a repository member.
              </p>
            </div>
            <div className="space-y-3 text-sm">
              <PermissionLine enabled label="Read private files" />
              <PermissionLine
                enabled={invite.permissions.can_push}
                label="Push changes"
              />
            </div>
          </section>

          <section className="grid gap-4 py-5 md:grid-cols-[220px_minmax(0,1fr)]">
            <div>
              <div className="text-sm font-semibold leading-5">Continue</div>
              <p className="mt-1 text-sm leading-5 text-muted-foreground">
                Use the email address this invite was sent to.
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              {isLoaded && isSignedIn ? (
                <Button
                  disabled={accepting}
                  onClick={() => void onAccept({ token })}
                  type="button"
                >
                  {accepting ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : (
                    <Check className="size-3.5" />
                  )}
                  <span>Accept invite</span>
                </Button>
              ) : (
                <>
                  <Button asChild>
                    <Link
                      params={{ _splat: '' }}
                      search={{ redirect_url: returnPath }}
                      to="/sign-up/$"
                    >
                      Create account
                    </Link>
                  </Button>
                  <Button asChild variant="secondary">
                    <Link
                      params={{ _splat: '' }}
                      search={{ redirect_url: returnPath }}
                      to="/sign-in/$"
                    >
                      Sign in
                    </Link>
                  </Button>
                </>
              )}
            </div>
          </section>
        </div>

        {acceptError && (
          <PageErrorAlert title="Invite acceptance failed">
            {acceptError}
          </PageErrorAlert>
        )}
      </PageContent>
    </AppShell>
  )
}

function PermissionLine({
  enabled,
  label,
}: {
  enabled: boolean
  label: string
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <span>{label}</span>
      <Badge variant={enabled ? 'success' : 'neutral'}>
        {enabled ? 'On' : 'Off'}
      </Badge>
    </div>
  )
}
