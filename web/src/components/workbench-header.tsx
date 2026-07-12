import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

export function WorkbenchHeader({
  actions,
  className,
  count,
  description,
  eyebrow,
  title,
}: {
  actions?: ReactNode
  className?: string
  count?: ReactNode
  description?: ReactNode
  eyebrow?: ReactNode
  title: ReactNode
}) {
  return (
    <header
      className={cn(
        'flex flex-col gap-4 border-b border-border px-4 pb-5 pt-7 sm:flex-row sm:items-end sm:justify-between sm:px-6 lg:px-8 lg:pt-9',
        className,
      )}
    >
      <div className="min-w-0">
        {eyebrow && (
          <div className="mb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.18em] text-muted-foreground">
            {eyebrow}
          </div>
        )}
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-4 gap-y-1">
          <h1 className="break-words text-2xl font-semibold leading-8 tracking-[-0.03em] sm:text-[30px]">
            {title}
          </h1>
          {count !== undefined && count !== null ? (
            <div className="text-sm text-muted-foreground">{count}</div>
          ) : null}
        </div>
        {description && (
          <p className="mt-2 max-w-3xl text-sm leading-6 text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap gap-2">{actions}</div>}
    </header>
  )
}
