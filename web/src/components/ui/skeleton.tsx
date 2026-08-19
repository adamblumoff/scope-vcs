import { cn } from '@/lib/utils'
import type { ComponentProps } from 'react'

export function Skeleton({
  className,
  ...props
}: ComponentProps<'div'>) {
  return (
    <div
      aria-hidden="true"
      className={cn('scope-skeleton rounded-md bg-muted', className)}
      data-slot="skeleton"
      {...props}
    />
  )
}
