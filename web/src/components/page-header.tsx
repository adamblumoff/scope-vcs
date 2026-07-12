import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

export function PageContent({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <section
      className={cn('mx-auto max-w-[1200px] px-4 py-7 sm:px-6 lg:px-8 lg:py-10', className)}
    >
      {children}
    </section>
  )
}

export function PageHeader({
  actions,
  badges,
  children,
  description,
  title,
  titleClassName,
}: {
  actions?: ReactNode
  badges?: ReactNode
  children?: ReactNode
  description?: ReactNode
  title: ReactNode
  titleClassName?: string
}) {
  return (
    <header className="flex flex-col gap-4 border-b border-border pb-6 md:flex-row md:items-end md:justify-between">
      <div className="min-w-0">
        {badges && (
          <div className="mb-3 flex flex-wrap items-center gap-2">
            {badges}
          </div>
        )}
        <h1
          className={cn(
            'break-words text-2xl font-semibold leading-8 tracking-[-0.03em] sm:text-[30px] sm:leading-10',
            titleClassName,
          )}
        >
          {title}
        </h1>
        {description && (
          <p className="mt-2.5 max-w-[680px] text-[15px] leading-6 text-muted-foreground">
            {description}
          </p>
        )}
        {children}
      </div>
      {actions && (
        <div className="flex w-full shrink-0 flex-wrap items-center justify-end gap-2 md:w-auto">
          {actions}
        </div>
      )}
    </header>
  )
}
