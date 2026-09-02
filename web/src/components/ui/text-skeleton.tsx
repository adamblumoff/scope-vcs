import { cn } from '@/lib/utils'
import type { ComponentProps } from 'react'

const TEXT_HEIGHT = {
  body: 'h-4',
  heading: 'h-8',
  meta: 'h-3',
  title: 'h-5',
} as const

const TEXT_WIDTH = {
  long: '32ch',
  medium: '20ch',
  short: '12ch',
  tiny: '4ch',
  xlong: '48ch',
} as const

export type TextSkeletonLength = keyof typeof TEXT_WIDTH

export function TextSkeleton({
  className,
  length = 'medium',
  size = 'body',
  ...props
}: Omit<ComponentProps<'span'>, 'style'> & {
  length?: TextSkeletonLength
  size?: keyof typeof TEXT_HEIGHT
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'scope-skeleton block max-w-full rounded-md bg-muted',
        TEXT_HEIGHT[size],
        className,
      )}
      data-skeleton-kind="text"
      data-slot="skeleton"
      style={{ width: TEXT_WIDTH[length] }}
      {...props}
    />
  )
}
