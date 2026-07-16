import { cn } from '@/lib/utils'

export type RequestReviewView = 'discussion' | 'changes' | 'activity'

const items: Array<{ label: string; value: RequestReviewView }> = [
  { label: 'Discussion', value: 'discussion' },
  { label: 'Changes', value: 'changes' },
  { label: 'Activity', value: 'activity' },
]

export function RequestReviewNavigation({
  onChange,
  view,
}: {
  onChange: (view: RequestReviewView) => void
  view: RequestReviewView
}) {
  return (
    <nav aria-label="Request review" className="mt-7 border-b border-border">
      <div className="flex gap-5 overflow-x-auto">
        {items.map((item) => (
          <button
            aria-current={view === item.value ? 'page' : undefined}
            className={cn(
              'min-h-11 shrink-0 border-b-2 px-0.5 text-sm font-medium',
              view === item.value
                ? 'border-foreground text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground',
            )}
            key={item.value}
            onClick={() => onChange(item.value)}
            type="button"
          >
            {item.label}
          </button>
        ))}
      </div>
    </nav>
  )
}
