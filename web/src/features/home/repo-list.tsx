import type { CliInstallCommands, RepoSummary } from '@/api/types'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { EmptyState } from '@/components/empty-state'
import { Link } from '@tanstack/react-router'
import { GitBranch } from 'lucide-react'

export function RepoList({
  cliInstallCommands,
  isOwner,
  repositories,
}: {
  cliInstallCommands: CliInstallCommands
  isOwner: boolean
  repositories: RepoSummary[]
}) {
  if (repositories.length === 0) {
    return (
      <EmptyState
        action={isOwner ? (
          <div className="mt-1 w-full max-w-[460px] space-y-2.5 text-left">
            <CopyableCodeBlock
              copyLabel="Copy macOS/Linux install command"
              value={cliInstallCommands.posix}
            />
            <CopyableCodeBlock
              copyLabel="Copy Windows install command"
              value={cliInstallCommands.windows}
            />
            <CopyableCodeBlock copyLabel="Copy init command" value="scope init" />
            <CopyableCodeBlock copyLabel="Copy push command" value="scope push" />
          </div>
        ) : undefined}
        className="mt-6"
        description={isOwner
          ? 'Install the CLI, then initialize an existing Git repository with at least one commit.'
          : 'This user does not have any repositories with public project files.'}
        icon={<GitBranch />}
        title={isOwner ? 'No repositories yet' : 'No repositories to show'}
      />
    )
  }

  return (
    <ul className="mt-6 divide-y divide-border">
      {repositories.map((repo) => (
        <li key={repo.id}>
          <RepoListRow isOwner={isOwner} repo={repo} />
        </li>
      ))}
    </ul>
  )
}

function RepoListRow({ isOwner, repo }: { isOwner: boolean; repo: RepoSummary }) {
  const showLifecycle = repo.lifecycle_state !== 'Ready'

  return (
    <div className="group relative flex items-center gap-3 rounded-md py-3 transition-colors hover:bg-accent/50">
      <div className="min-w-0 flex-1">
        <Link
          className="font-mono text-sm leading-5 outline-none after:absolute after:inset-0"
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/$owner/$repo"
        >
          <span className="text-muted-foreground">{repo.owner_handle}/</span>
          <span className="font-semibold text-foreground">{repo.name}</span>
        </Link>
        {(isOwner && showLifecycle) || repo.access.actor !== 'Public' ? (
          <div className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
            {isOwner && showLifecycle && (
              <LifecycleBadge state={repo.lifecycle_state} />
            )}
            {repo.access.actor !== 'Public' && <span>{repo.access.actor}</span>}
          </div>
        ) : null}
      </div>

      {/* The whole row already links to the repo, so only surface an action
          when it says something the row does not (e.g. "Clean up"). */}
      <div className="relative z-10 flex shrink-0 items-center">
        <RepoPrimaryActionButton
          includeOpen={false}
          repo={repo}
          variant="secondary"
        />
      </div>
    </div>
  )
}
