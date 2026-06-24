import type { RepoSummary } from '@/api/types'
import {
  type RepoAttentionAction,
  repoAttentionAction,
} from '@/components/repo-primary-action'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { Link } from '@tanstack/react-router'
import {
  GitBranch,
  GitPullRequestArrow,
  LogIn,
  Rocket,
  Upload,
} from 'lucide-react'

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
              {signedIn
                ? 'Create a repository to start.'
                : 'Sign in to start from an empty workspace.'}
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
    <TooltipProvider>
      <div className="mt-6 border-t border-border">
        <div className="hidden border-b border-border px-2 py-2 text-xs font-medium leading-4 text-muted-foreground lg:block">
          <div>Repository</div>
        </div>
        {repositories.map((repo) => (
          <RepoListRow key={repo.id} repo={repo} />
        ))}
      </div>
    </TooltipProvider>
  )
}

function RepoListRow({ repo }: { repo: RepoSummary }) {
  const action = repoAttentionAction(repo)

  return (
    <div className="border-b border-border px-2 py-3 text-sm transition-colors last:border-b-0 hover:bg-muted/40 lg:min-h-[60px]">
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-1.5">
          <Link
            className="min-w-0 truncate font-mono text-sm font-semibold leading-5 underline-offset-4 hover:underline"
            params={{ owner: repo.owner_handle, repo: repo.name }}
            to="/repos/$owner/$repo"
          >
            {repo.id}
          </Link>
          {action && <RepoAttentionActionLink action={action} repo={repo} />}
        </div>
        <div className="mt-1 flex flex-wrap gap-2 text-xs leading-4 text-muted-foreground lg:hidden">
          <span>{repo.role ?? 'reader'}</span>
        </div>
        <div className="mt-1 hidden text-xs leading-4 text-muted-foreground lg:block">
          {repo.role && <span>{repo.role}</span>}
        </div>
      </div>
    </div>
  )
}

function RepoAttentionActionLink({
  action,
  repo,
}: {
  action: RepoAttentionAction
  repo: RepoSummary
}) {
  const Icon = attentionActionIcon(action)

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={action.label}
          asChild
          className="size-5 rounded-md text-muted-foreground hover:text-foreground"
          size="icon-xs"
          variant="ghost"
        >
          <Link
            params={{ owner: repo.owner_handle, repo: repo.name }}
            to={action.to}
          >
            <Icon className="size-3.5" />
          </Link>
        </Button>
      </TooltipTrigger>
      <TooltipContent>{action.label}</TooltipContent>
    </Tooltip>
  )
}

function attentionActionIcon(action: RepoAttentionAction) {
  switch (action.icon) {
    case 'publish-review':
      return Rocket
    case 'setup':
      return Upload
    case 'update-review':
      return GitPullRequestArrow
  }
}
