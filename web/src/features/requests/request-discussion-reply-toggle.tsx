import { Button } from '@/components/ui/button'
import { MessageSquare } from 'lucide-react'

/**
 * One reply disclosure control at every depth. Threads collapse to their
 * latest preview, while nested branches hide their children completely.
 */
export function RequestDiscussionReplyToggle({
  count,
  expanded,
  keepsPreview = false,
  onToggle,
  subtle = false,
}: {
  count: number
  expanded: boolean
  keepsPreview?: boolean
  onToggle: () => void
  subtle?: boolean
}) {
  const countLabel = `${count} ${count === 1 ? 'reply' : 'replies'}`
  let label = `Show ${countLabel}`
  if (keepsPreview) {
    label = expanded ? 'Show fewer replies' : `Show all ${countLabel}`
  } else if (expanded) {
    label = 'Hide replies'
  }

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
