import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

/**
 * The one empty-state treatment. `inline` is for empty regions inside a
 * populated page (a rail section, a list slot); the default fills a pane.
 */
export function EmptyState({
  action,
  className,
  description,
  icon,
  inline = false,
  title,
}: {
  action?: ReactNode
  className?: string
  description?: ReactNode
  icon?: ReactNode
  inline?: boolean
  title: ReactNode
}) {
  if (inline) {
    return (
      <p className={cn('text-sm leading-5 text-muted-foreground', className)}>
        {title}
      </p>
    )
  }

  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center gap-3 px-6 py-16 text-center',
        className,
      )}
    >
      {icon && (
        <span className="text-muted-foreground [&_svg]:size-5">{icon}</span>
      )}
      <div>
        <p className="text-sm font-semibold leading-5">{title}</p>
        {description && (
          <p className="mt-1 max-w-[46ch] text-sm leading-5 text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      {action}
    </div>
  )
}

/**
 * Centered status for a workbench pane: idle prompts, errors and retries.
 * Replaces the per-feature copies that each picked their own min-height.
 */
export function PanelState({
  children,
  className,
  role,
  tone = 'muted',
}: {
  children: ReactNode
  className?: string
  role?: 'alert'
  tone?: 'error' | 'muted'
}) {
  return (
    <div
      className={cn(
        'flex h-full min-h-[220px] flex-col items-center justify-center gap-3 px-6 py-10 text-center text-sm leading-5',
        tone === 'error' ? 'text-danger-strong' : 'text-muted-foreground',
        className,
      )}
      role={role}
    >
      {children}
    </div>
  )
}
