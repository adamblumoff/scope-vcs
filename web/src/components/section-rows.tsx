import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

type SectionRowColumns = 'default' | 'compact'

const columnClass: Record<SectionRowColumns, string> = {
  compact: 'md:grid-cols-[180px_minmax(0,1fr)]',
  default: 'md:grid-cols-[240px_minmax(0,1fr)]',
}

export function SectionRows({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        'mt-8 divide-y divide-border border-y border-border',
        className,
      )}
    >
      {children}
    </div>
  )
}

export function SectionRow({
  children,
  columns = 'default',
  description,
  icon,
  title,
}: {
  children: ReactNode
  columns?: SectionRowColumns
  description?: ReactNode
  icon?: ReactNode
  title: ReactNode
}) {
  return (
    <section className={cn('grid gap-4 py-5', columnClass[columns])}>
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-sm font-semibold leading-5">
          {icon}
          <span>{title}</span>
        </div>
        {description && (
          <p className="mt-1 text-sm leading-5 text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      <div className="min-w-0 md:pt-0.5">{children}</div>
    </section>
  )
}
