import type { CommitSummary } from '@/api/types'
import { historyRowLabels } from '@/features/history/history-row-labels'
import { cn } from '@/lib/utils'
import { GitCommit } from 'lucide-react'

export function CommitList({
  commits,
  onSelectCommit,
  selectedCommitId,
}: {
  commits: CommitSummary[]
  onSelectCommit: (commit: CommitSummary) => void
  selectedCommitId: string | null
}) {
  return (
    <div className="border-b border-border py-2 lg:border-b-0 lg:border-r">
      {commits.map((commit) => {
        const labels = historyRowLabels(commit)
        const selected = selectedCommitId === commit.projected_id
        return (
          <button
            aria-label={labels.ariaLabel}
            className={cn(
              'grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 px-5 py-2.5 text-left text-sm transition-colors sm:px-6 lg:px-8',
              selected
                ? 'bg-accent shadow-[inset_2px_0_0_0_var(--brand)]'
                : 'hover:bg-accent/50',
            )}
            key={commit.projected_id}
            onClick={() => onSelectCommit(commit)}
            title={commit.logical_commit_id}
            type="button"
          >
            <span className="flex min-w-0 items-center gap-2">
              <GitCommit className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0">
                <span className="block truncate text-[13px] font-medium">
                  {labels.title}
                </span>
                <span className="mt-0.5 block truncate font-mono text-[11px] leading-4 text-muted-foreground">
                  {labels.compactId}
                </span>
              </span>
            </span>
            <span className="font-mono text-xs tabular-nums text-muted-foreground">
              {commit.change_count}
            </span>
          </button>
        )
      })}
    </div>
  )
}
