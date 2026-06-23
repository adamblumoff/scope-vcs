import type { ProjectionPreview } from '@/api/types'
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
  return (
    <div className="mb-4 grid gap-3 border-y border-border py-3 text-sm sm:grid-cols-3">
      <Metric
        label="Visible"
        value={fileCountLabel(preview.summary.visible_files)}
      />
      {preview.audience === 'public' && showPrivateCounts ? (
        <Metric
          label="Private left out"
          value={fileCountLabel(preview.summary.hidden_files)}
        />
      ) : (
        <Metric
          label="Audience"
          value={audienceLabel(preview.audience)}
        />
      )}
      <Metric
        label="History"
        value={commitCountLabel(preview.summary.visible_commits)}
      />
    </div>
  )
}

function Metric({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="min-w-0">
      <div className="text-xs leading-4 text-muted-foreground">{label}</div>
      <div className="mt-1 truncate font-mono text-sm font-semibold leading-5">
        {value}
      </div>
    </div>
  )
}
