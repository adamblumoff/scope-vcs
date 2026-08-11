import { cn } from '@/lib/utils'

export function RunStatusDot({
  className,
  state,
}: {
  className?: string
  state: string
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'size-2 shrink-0 rounded-full',
        ['online', 'running', 'succeeded'].includes(state) && 'bg-success',
        ['blocked', 'canceling', 'queued', 'leased', 'pending'].includes(state) &&
          'bg-warning',
        ['failed', 'lost', 'offline'].includes(state) && 'bg-danger',
        ['canceled', 'disabled', 'skipped'].includes(state) && 'bg-muted-foreground',
        className,
      )}
    />
  )
}
