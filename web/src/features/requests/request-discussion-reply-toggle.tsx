import { Button } from '@/components/ui/button'
import { MessageSquare } from 'lucide-react'

/**
 * The only control that shows or hides replies, at every depth. The verb
 * carries the state, so there is no icon that can contradict it. Inside a
 * reply it renders as quiet text, because there it sits beside the body copy
 * rather than in a row of primary actions.
 */
export function RequestDiscussionReplyToggle({
  count,
  expanded,
  onToggle,
  subtle = false,
}: {
  count: number
  expanded: boolean
  onToggle: () => void
  subtle?: boolean
}) {
  const label = expanded
    ? 'Hide replies'
    : `Show ${count} ${count === 1 ? 'reply' : 'replies'}`

  if (subtle) {
    return (
      <button
        aria-expanded={expanded}
        className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground"
        onClick={onToggle}
        type="button"
      >
        <MessageSquare className="size-3" />
        {label}
      </button>
    )
  }

  return (
    <Button
      aria-expanded={expanded}
      onClick={onToggle}
      size="sm"
      type="button"
      variant="ghost"
    >
      <MessageSquare className="size-3.5" />
      {label}
    </Button>
  )
}
