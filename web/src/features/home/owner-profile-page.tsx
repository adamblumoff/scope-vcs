import type { ProfileState } from '@/api/types'
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { useHomeFlash } from '@/lib/home-flash'
import { UserButton } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { CheckCircle2, KeyRound } from 'lucide-react'
import { RepoList } from './repo-list'

export function OwnerProfilePage({ state }: { state: ProfileState }) {
  const flash = useHomeFlash()
  const { account, profile } = state
  const isOwner = account.user?.handle === profile.handle

  return (
    <AppShell
      header={() => (
        <ApplicationTopbar>
          {account.user ? (
            <>
              <Button
                aria-label="CLI sessions"
                asChild
                size="icon-sm"
                title="CLI sessions"
                type="button"
                variant="ghost"
              >
                <Link to="/account">
                  <KeyRound />
                </Link>
              </Button>
              <UserButton />
            </>
          ) : (
            <Button asChild size="sm" variant="secondary">
              <Link
                params={{ _splat: '' }}
                search={{ redirect_url: `/${profile.handle}` }}
                to="/sign-in/$"
              >
                Sign in
              </Link>
            </Button>
          )}
        </ApplicationTopbar>
      )}
    >
      <PageContent>
        <PageHeader title={`@${profile.handle}`} />

        {flash && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>Success</AlertTitle>
            <AlertDescription>{flash}</AlertDescription>
          </Alert>
        )}

        <RepoList
          cliInstallCommands={state.cliInstallCommands}
          isOwner={isOwner}
          repositories={profile.repositories}
        />
      </PageContent>
    </AppShell>
  )
}
