import type { CliInstallCommands, RepoSummary } from '@/api/types'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Link } from '@tanstack/react-router'
import { ChevronRight, GitBranch } from 'lucide-react'

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
      <div className="mt-8 flex flex-col items-center gap-4 border-y border-border px-6 py-14 text-center text-sm">
        <div className="flex size-10 items-center justify-center rounded-lg border border-[var(--border-strong)] bg-secondary text-[var(--platinum)] shadow-[var(--shadow-card)]">
          <GitBranch className="size-5" />
        </div>
        <div className="max-w-[420px]">
          <div className="text-base font-semibold leading-6">
            {isOwner ? 'No repositories yet' : 'No repositories to show'}
          </div>
          <p className="mt-1 leading-6 text-muted-foreground">
            {isOwner
              ? 'Install the CLI, then initialize an existing Git repository with at least one commit.'
              : 'This user does not have any repositories with public project files.'}
          </p>
        </div>
        {isOwner && <div className="mt-1 w-full max-w-[460px] space-y-2.5 text-left">
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
        </div>}
      </div>
    )
  }

  return (
    <div className="mt-7 border-y border-border">
      <div className="border-b border-border px-3 py-3 font-mono text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
        {repositories.length}{' '}
        {repositories.length === 1 ? 'repository' : 'repositories'}
      </div>
      <ul className="divide-y divide-border">
        {repositories.map((repo) => (
          <li key={repo.id}>
            <RepoListRow isOwner={isOwner} repo={repo} />
          </li>
        ))}
      </ul>
    </div>
  )
}

function RepoListRow({ isOwner, repo }: { isOwner: boolean; repo: RepoSummary }) {
  const showLifecycle = repo.lifecycle_state !== 'Ready'

  return (
    <div className="group relative flex min-h-16 items-center gap-3 px-3 py-3 transition-colors hover:bg-muted/45">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-[var(--border-strong)] bg-secondary text-[var(--platinum)] shadow-[var(--shadow-card)]">
        <GitBranch className="size-4" />
      </div>

      <div className="min-w-0 flex-1">
        <Link
          className="font-mono text-sm leading-5 tracking-tight outline-none after:absolute after:inset-0"
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/$owner/$repo"
        >
          <span className="text-muted-foreground">{repo.owner_handle}/</span>
          <span className="font-semibold text-foreground">
            {repo.name}
          </span>
        </Link>
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          {isOwner && showLifecycle && <LifecycleBadge state={repo.lifecycle_state} />}
          {repo.access.actor !== 'Public' && <span>{repo.access.actor}</span>}
        </div>
      </div>

      <div className="relative z-10 flex shrink-0 items-center gap-1">
        <RepoPrimaryActionButton repo={repo} variant="secondary" />
        <ChevronRight className="size-4 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5 group-hover:text-muted-foreground" />
      </div>
    </div>
  )
}
