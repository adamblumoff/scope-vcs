import { cn } from '@/lib/utils'
import type { ComponentProps } from 'react'

export function BlockSkeleton({
  className,
  ...props
}: Omit<ComponentProps<'div'>, 'style'>) {
  return (
    <div
      aria-hidden="true"
      className={cn('scope-skeleton max-w-full rounded-md bg-muted', className)}
      data-skeleton-kind="block"
      data-slot="skeleton"
      {...props}
    />
  )
}
