import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageRail } from '@/components/page-header'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

export function PendingSurface({
  children,
  className,
  delay = false,
  label = 'Loading page',
}: {
  children?: ReactNode
  className?: string
  delay?: boolean
  label?: string
}) {
  return (
    <output
      aria-busy="true"
      className={cn(
        'scope-pending-enter block min-h-full w-full',
        delay && 'scope-pending-delayed',
        className,
      )}
      data-slot="pending-surface"
    >
      <span className="sr-only">{label}</span>
      {children ?? <DefaultPageSkeleton />}
    </output>
  )
}

export function ApplicationPendingShell({
  children,
  contextLabel,
  label,
  repository,
}: {
  children?: ReactNode
  contextLabel?: string
  label: string
  repository?: { owner: string; repo: string }
}) {
  return (
    <AppShell
      header={() => (
        <ApplicationTopbar
          contextLabel={contextLabel}
          repository={repository}
        />
      )}
    >
      <PageRail className="min-h-full">
        <PendingSurface label={label}>{children}</PendingSurface>
      </PageRail>
    </AppShell>
  )
}

function DefaultPageSkeleton() {
  return (
    <div className="py-8 lg:py-10">
      <Skeleton className="h-8 w-52 max-w-2/3" />
      <Skeleton className="mt-3 h-4 w-96 max-w-full" />
      <div className="mt-8 divide-y divide-border border-y border-border">
        {[72, 58, 82, 64].map((width) => (
          <div className="py-5" key={width}>
            <Skeleton className="h-4" style={{ width: `${width}%` }} />
            <Skeleton className="mt-2 h-3 w-2/5" />
          </div>
        ))}
      </div>
    </div>
  )
}
