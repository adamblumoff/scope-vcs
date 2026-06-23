import type { RepoSummary } from '@/api/types'
import { lifecycleLabel } from '@/components/repo-lifecycle-label'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { GitBranch, LogIn } from 'lucide-react'

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
      <div className="mt-6 border-t border-border py-9">
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
    <div className="mt-6 border-t border-border">
      <div className="hidden grid-cols-[minmax(0,1fr)_160px_120px_96px] items-center gap-4 border-b border-border px-2 py-2 text-xs font-medium leading-4 text-muted-foreground lg:grid">
        <div>Repository</div>
        <div>Status</div>
        <div>Visibility</div>
        <div className="text-right">Action</div>
      </div>
      {repositories.map((repo) => (
        <div
          className="grid gap-3 border-b border-border px-2 py-3 text-sm transition-colors last:border-b-0 hover:bg-muted/40 lg:min-h-[60px] lg:grid-cols-[minmax(0,1fr)_160px_120px_96px] lg:items-center lg:gap-4"
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
          <div className="hidden min-h-8 items-center text-xs leading-4 text-muted-foreground lg:flex">
            {lifecycleLabel(repo.lifecycle_state)}
          </div>
          <div className="hidden min-h-8 items-center lg:flex">
            <VisibilityBadge visibility={repo.default_visibility} />
          </div>
          <div className="flex items-center gap-2 lg:justify-end">
            <RepoPrimaryActionButton repo={repo} />
          </div>
        </div>
      ))}
    </div>
  )
}
