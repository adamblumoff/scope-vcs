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
        visibility === 'Private' && 'border-red-500/60 bg-red-500/15 text-red-300',
        visibility === 'Public' && 'border-green-400 bg-green-100 text-green-900',
        visibility === 'Mixed' && 'border-yellow-500/60 bg-yellow-500/15 text-yellow-300',
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
