import type { CliInstallCommands, RepoSummary } from '@/api/types'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Link } from '@tanstack/react-router'
import { ChevronRight, GitBranch } from 'lucide-react'

export function RepoList({
  cliInstallCommands,
  repositories,
}: {
  cliInstallCommands: CliInstallCommands
  repositories: RepoSummary[]
}) {
  if (repositories.length === 0) {
    return (
      <div className="mt-8 flex flex-col items-center gap-4 rounded-2xl border border-dashed border-border px-6 py-12 text-center text-sm">
        <div className="flex size-11 items-center justify-center rounded-xl bg-brand-muted text-brand">
          <GitBranch className="size-5" />
        </div>
        <div className="max-w-[420px]">
          <div className="text-base font-semibold leading-6">
            No repositories yet
          </div>
          <p className="mt-1 leading-6 text-muted-foreground">
            Install the CLI, then initialize this folder from your terminal to
            create your first repository.
          </p>
        </div>
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
        </div>
      </div>
    )
  }

  return (
    <div className="mt-6">
      <div className="mb-2 px-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {repositories.length}{' '}
        {repositories.length === 1 ? 'repository' : 'repositories'}
      </div>
      <ul className="flex flex-col gap-1">
        {repositories.map((repo) => (
          <li key={repo.id}>
            <RepoListRow repo={repo} />
          </li>
        ))}
      </ul>
    </div>
  )
}

function RepoListRow({ repo }: { repo: RepoSummary }) {
  const showLifecycle = repo.lifecycle_state !== 'Published'

  return (
    <div className="group relative flex items-center gap-3 rounded-xl border border-transparent px-3 py-3 transition-colors hover:border-border hover:bg-muted/40">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-border bg-card text-muted-foreground transition-colors group-hover:text-brand">
        <GitBranch className="size-4" />
      </div>

      <div className="min-w-0 flex-1">
        <Link
          className="font-mono text-sm leading-5 tracking-tight outline-none after:absolute after:inset-0 after:rounded-xl"
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/repos/$owner/$repo"
        >
          <span className="text-muted-foreground">{repo.owner_handle}/</span>
          <span className="font-semibold text-foreground group-hover:text-brand">
            {repo.name}
          </span>
        </Link>
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <VisibilityBadge compact visibility={repo.default_visibility} />
          {showLifecycle && <LifecycleBadge state={repo.lifecycle_state} />}
          <span>{repo.access.actor}</span>
        </div>
      </div>

      <div className="relative z-10 flex shrink-0 items-center gap-1">
        <RepoPrimaryActionButton repo={repo} variant="secondary" />
        <ChevronRight className="size-4 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5 group-hover:text-muted-foreground" />
      </div>
    </div>
  )
}
