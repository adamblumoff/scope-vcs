import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { Visibility, VisibilityState } from '@/api/types'
import { Globe2, Lock, Minus } from 'lucide-react'

export function VisibilityBadge({
  compact = false,
  visibility,
}: {
  compact?: boolean
  visibility: Visibility | VisibilityState
}) {
  return (
    <Badge
      aria-label={compact ? `${visibility} visibility` : undefined}
      className={cn(compact && 'w-5 gap-0 px-0')}
      title={compact ? `${visibility} visibility` : undefined}
      variant={
        visibility === 'Private'
          ? 'danger'
          : visibility === 'Public'
            ? 'success'
            : 'warning'
      }
    >
      {visibility === 'Mixed' ? (
        <Minus className="size-3" />
      ) : visibility === 'Private' ? (
        <Lock className="size-3" />
      ) : (
        <Globe2 className="size-3" />
      )}
      {!compact && visibility}
    </Badge>
  )
}
