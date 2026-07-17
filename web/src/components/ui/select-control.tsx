import { cn } from '@/lib/utils'
import { ChevronDown } from 'lucide-react'
import type { ComponentProps } from 'react'

export function SelectControl({
  className,
  containerClassName,
  children,
  ...props
}: ComponentProps<'select'> & { containerClassName?: string }) {
  return (
    <span className={cn('relative inline-flex', containerClassName)}>
      <select
        className={cn(
          'h-9 appearance-none rounded-md border border-input bg-background',
          'pl-3 pr-9 text-sm outline-none',
          'focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50',
          className,
        )}
        {...props}
      >
        {children}
      </select>
      <ChevronDown
        aria-hidden="true"
        className="pointer-events-none absolute right-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
      />
    </span>
  )
}
