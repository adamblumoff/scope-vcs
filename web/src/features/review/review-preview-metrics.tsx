import type { ProjectionPreview } from '@/api/types'
import { GitCommit, Lock, UserRoundCheck } from 'lucide-react'
import type { ReactNode } from 'react'
import {
  audienceLabel,
  commitCountLabel,
  fileCountLabel,
} from './review-labels'

export function ReviewPreviewMetrics({
  preview,
  showPrivateCounts,
}: {
  preview: ProjectionPreview
  showPrivateCounts: boolean
}) {
  const visibleFiles = fileCountLabel(preview.summary.visible_files)
  const hiddenFiles = fileCountLabel(preview.summary.hidden_files)
  const visibleCommits = commitCountLabel(preview.summary.visible_commits)
  const audience = audienceLabel(preview.audience)

  return (
    <div className="grid grid-cols-3 gap-1 text-xs sm:min-w-[300px]">
      <SummaryChip
        icon={<UserRoundCheck className="size-3" />}
        label="Shown"
        value={visibleFiles}
      />
      {preview.audience === 'public' && showPrivateCounts ? (
        <SummaryChip
          icon={<Lock className="size-3" />}
          label="Excluded"
          tone="private"
          value={hiddenFiles}
        />
      ) : (
        <SummaryChip
          icon={<UserRoundCheck className="size-3" />}
          label="Audience"
          value={audience}
        />
      )}
      <SummaryChip
        icon={<GitCommit className="size-3" />}
        label="History"
        value={visibleCommits}
      />
    </div>
  )
}

function SummaryChip({
  icon,
  label,
  tone = 'default',
  value,
}: {
  icon: ReactNode
  label: string
  tone?: 'default' | 'private'
  value: number | string
}) {
  return (
    <div
      className={
        tone === 'private'
          ? 'min-w-0 border-l-2 border-white bg-red-100/35 px-2 py-2.5 dark:bg-red-100/20'
          : 'min-w-0 border-l-2 border-white bg-background/50 px-2 py-2.5 dark:bg-background/35'
      }
    >
      <div className="flex items-center gap-1 text-[11px] leading-4 text-muted-foreground">
        {icon}
        <span>{label}</span>
      </div>
      <div
        className={
          tone === 'private'
            ? 'mt-1.5 truncate font-mono text-xs font-semibold leading-4 text-red-900'
            : 'mt-1.5 truncate font-mono text-xs font-semibold leading-4'
        }
      >
        {value}
      </div>
    </div>
  )
}
