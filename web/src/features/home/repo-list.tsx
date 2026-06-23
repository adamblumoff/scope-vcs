import type { RepoSummary } from '@/api/types'
import { lifecycleLabel } from '@/components/repo-lifecycle-label'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowRight, LogIn } from 'lucide-react'

export function RepoList({
  onSignIn,
  repositories,
  signedIn,
}: {
  onSignIn: () => Promise<void>
  repositories: RepoSummary[]
  signedIn: boolean
}) {
  if (repositories.length === 0) {
    return (
      <div className="mt-8 border-y border-border">
        <div className="grid gap-2 py-10 text-sm sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
          <div className="min-w-0">
            <div className="font-medium leading-5">No repositories</div>
            <div className="mt-1 leading-5 text-muted-foreground">
              {signedIn
                ? 'Create a repository to start.'
                : 'Sign in to start from an empty workspace.'}
            </div>
          </div>
          {!signedIn && (
            <Button size="sm" onClick={() => void onSignIn()} type="button">
              <LogIn className="size-3.5" />
              <span>Sign in</span>
            </Button>
          )}
        </div>
      </div>
    )
  }

  return (
    <div className="mt-8 divide-y divide-border border-y border-border">
      {repositories.map((repo) => (
        <div
          className="grid gap-3 py-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
          key={repo.id}
        >
          <div className="min-w-0">
            <Link
              className="block truncate font-mono text-sm font-semibold leading-5 underline-offset-4 hover:underline"
              params={{ owner: repo.owner_handle, repo: repo.name }}
              to="/repos/$owner/$repo"
            >
              {repo.id}
            </Link>
            <div className="mt-1 flex flex-wrap gap-2 text-xs leading-4 text-muted-foreground">
              <span>{lifecycleLabel(repo.lifecycle_state)}</span>
              {repo.role && <span>{repo.role}</span>}
            </div>
          </div>
          <div className="flex items-center gap-2 sm:justify-end">
            <VisibilityBadge visibility={repo.default_visibility} />
            <Button asChild size="sm" variant="secondary">
              <Link
                params={{ owner: repo.owner_handle, repo: repo.name }}
                to="/repos/$owner/$repo"
              >
                <ArrowRight className="size-3.5" />
                <span>Open</span>
              </Link>
            </Button>
            {repo.lifecycle_state === 'PendingFirstPush' && (
              <Button asChild size="sm" variant="secondary">
                <Link
                  params={{ owner: repo.owner_handle, repo: repo.name }}
                  to="/repos/$owner/$repo/setup"
                >
                  <ArrowRight className="size-3.5" />
                  <span>Setup</span>
                </Link>
              </Button>
            )}
            {(repo.lifecycle_state === 'PendingPublish' ||
              (repo.lifecycle_state === 'Published' && repo.staged_update_pending)) && (
              <Button asChild size="sm" variant="secondary">
                <Link
                  params={{ owner: repo.owner_handle, repo: repo.name }}
                  to="/repos/$owner/$repo/review"
                >
                  <ArrowRight className="size-3.5" />
                  <span>Review</span>
                </Link>
              </Button>
            )}
          </div>
        </div>
      ))}
    </div>
  )
}
