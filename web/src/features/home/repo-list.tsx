import type { RepoSummary } from '@/api/types'
import { lifecycleLabel } from '@/components/repo-lifecycle-label'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowRight, GitBranch, LogIn } from 'lucide-react'

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
      <div className="mt-6 border-y border-border py-9">
        <div className="mx-auto flex max-w-[520px] flex-col items-center gap-3 text-center text-sm">
          <div className="flex size-9 items-center justify-center rounded-md border border-border text-muted-foreground">
            <GitBranch className="size-4" />
          </div>
          <div>
            <div className="font-medium leading-5">No repositories</div>
            <p className="mt-1 leading-5 text-muted-foreground">
              {signedIn ? 'Create a repository to start.' : 'Sign in to start from an empty workspace.'}
            </p>
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
    <div className="mt-6 divide-y divide-border border-y border-border">
      <div className="hidden grid-cols-[minmax(0,1.4fr)_130px_110px_auto] gap-3 px-2 py-2 text-xs font-medium leading-4 text-muted-foreground lg:grid">
        <div>Repository</div>
        <div>Status</div>
        <div>Visibility</div>
        <div className="text-right">Action</div>
      </div>
      {repositories.map((repo) => (
        <div
          className="grid gap-3 px-2 py-3 text-sm transition-colors hover:bg-muted/40 lg:grid-cols-[minmax(0,1.4fr)_130px_110px_auto] lg:items-center"
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
            <div className="mt-1 flex flex-wrap gap-2 text-xs leading-4 text-muted-foreground lg:hidden">
              <span>{repo.role ?? 'reader'}</span>
              <span>{lifecycleLabel(repo.lifecycle_state)}</span>
              <VisibilityBadge visibility={repo.default_visibility} />
            </div>
            <div className="mt-1 hidden text-xs leading-4 text-muted-foreground lg:block">
              {repo.role && <span>{repo.role}</span>}
            </div>
          </div>
          <div className="hidden text-xs leading-4 text-muted-foreground lg:block">
            {lifecycleLabel(repo.lifecycle_state)}
          </div>
          <div className="hidden lg:block">
            <VisibilityBadge visibility={repo.default_visibility} />
          </div>
          <div className="flex items-center gap-2 lg:justify-end">
            <RepoActions repo={repo} />
          </div>
        </div>
      ))}
    </div>
  )
}

function RepoActions({ repo }: { repo: RepoSummary }) {
  const action =
    repo.lifecycle_state === 'PendingFirstPush'
      ? { label: 'Setup', to: '/repos/$owner/$repo/setup' as const }
      : repo.lifecycle_state === 'PendingPublish' ||
          (repo.lifecycle_state === 'Published' && repo.staged_update_pending)
        ? { label: 'Review', to: '/repos/$owner/$repo/review' as const }
        : { label: 'Open', to: '/repos/$owner/$repo' as const }

  return (
    <Button asChild size="sm" variant="secondary">
      <Link
        params={{ owner: repo.owner_handle, repo: repo.name }}
        to={action.to}
      >
        <ArrowRight className="size-3.5" />
        <span>{action.label}</span>
      </Link>
    </Button>
  )
}
