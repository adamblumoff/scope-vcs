import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { Visibility, VisibilityState } from '@/api/types'
import { Globe2, Lock, Minus } from 'lucide-react'

export function VisibilityBadge({
  visibility,
}: {
  visibility: Visibility | VisibilityState
}) {
  return (
    <Badge
      className={cn(
        visibility === 'Private' && 'border-amber-400 bg-amber-100 text-amber-900',
        visibility === 'Public' && 'border-green-400 bg-green-100 text-green-900',
        visibility === 'Mixed' && 'border-blue-400 bg-blue-100 text-blue-900',
      )}
      variant="outline"
    >
      {visibility === 'Mixed' ? (
        <Minus className="size-3" />
      ) : visibility === 'Private' ? (
        <Lock className="size-3" />
      ) : (
        <Globe2 className="size-3" />
      )}
      {visibility}
    </Badge>
  )
}
