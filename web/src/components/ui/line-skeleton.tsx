import { cn } from '@/lib/utils'
import type { ComponentProps } from 'react'

const LINE_WIDTH = {
  full: '100%',
  long: '85%',
  medium: '65%',
  short: '45%',
} as const

export type LineSkeletonLength = keyof typeof LINE_WIDTH

export function LineSkeleton({
  className,
  length = 'medium',
  ...props
}: Omit<ComponentProps<'span'>, 'style'> & {
  length?: LineSkeletonLength
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'scope-skeleton block h-3 max-w-full rounded-md bg-muted',
        className,
      )}
      data-skeleton-kind="line"
      data-slot="skeleton"
      style={{ width: LINE_WIDTH[length] }}
      {...props}
    />
  )
}
