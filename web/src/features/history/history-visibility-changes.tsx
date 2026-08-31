import type { HistoryEntryDetail } from '@/api/types'
import { VisibilityBadge } from '@/components/visibility-badge'
import { ArrowRight } from 'lucide-react'

export function VisibilityChanges({
  changes,
}: {
  changes: HistoryEntryDetail['visibility_changes']
}) {
  return (
    <section aria-labelledby="history-visibility-changes" className="border-b border-border">
      <div className="px-5 pb-2 pt-4 sm:px-6">
        <h4
          className="text-xs font-semibold uppercase tracking-wide text-muted-foreground"
          id="history-visibility-changes"
        >
          Visibility changes
        </h4>
      </div>
      <div className="divide-y divide-border">
        {changes.map((change) => (
          <div className="flex min-h-10 items-center gap-2 px-5 py-2 sm:px-6" key={change.path}>
            <span className="min-w-0 flex-1 truncate font-mono text-xs" title={change.path}>
              {change.path}
            </span>
            <VisibilityBadge compact visibility={change.old_visibility} />
            <ArrowRight aria-hidden className="size-3 shrink-0 text-muted-foreground" />
            <VisibilityBadge compact visibility={change.new_visibility} />
          </div>
        ))}
      </div>
    </section>
  )
}
