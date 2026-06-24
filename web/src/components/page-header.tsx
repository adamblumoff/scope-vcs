import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

type RenderSlot = ReactNode | (() => ReactNode)

export function PageContent({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <section
      className={cn('mx-auto max-w-[1320px] px-4 py-6 sm:px-6 lg:py-8', className)}
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
  actions?: () => ReactNode
  badges?: () => ReactNode
  children?: ReactNode
  description?: RenderSlot
  title: ReactNode
  titleClassName?: string
}) {
  const renderedDescription = renderSlot(description)

  return (
    <div className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
      <div className="min-w-0">
        {badges && (
          <div className="mb-3 flex flex-wrap items-center gap-2">
            {badges()}
          </div>
        )}
        <h1
          className={cn(
            'truncate text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10',
            titleClassName,
          )}
        >
          {title}
        </h1>
        {renderedDescription && (
          <p className="mt-2 max-w-[680px] text-sm leading-5 text-muted-foreground">
            {renderedDescription}
          </p>
        )}
        {children}
      </div>
      {actions && (
        <div className="flex w-full shrink-0 flex-wrap items-center justify-end gap-2 md:w-auto">
          {actions()}
        </div>
      )}
    </div>
  )
}

function renderSlot(slot: RenderSlot | undefined) {
  return typeof slot === 'function' ? slot() : slot
}
