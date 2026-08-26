import { Button } from '@/components/ui/button'
import { MessageSquare } from 'lucide-react'

/**
 * The only control that shows or hides replies, at every depth. The verb
 * carries the state, so there is no icon that can contradict it.
 */
export function RequestDiscussionReplyToggle({
  count,
  expanded,
  onToggle,
}: {
  count: number
  expanded: boolean
  onToggle: () => void
}) {
  return (
    <Button
      aria-expanded={expanded}
      onClick={onToggle}
      size="sm"
      type="button"
      variant="ghost"
    >
      <MessageSquare className="size-3.5" />
      {expanded
        ? 'Hide replies'
        : `Show ${count} ${count === 1 ? 'reply' : 'replies'}`}
    </Button>
  )
}
