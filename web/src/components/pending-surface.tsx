import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageRail } from '@/components/page-header'
import {
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

const DEFAULT_ROWS: { id: string; length: TextSkeletonLength }[] = [
  { id: 'primary', length: 'long' },
  { id: 'secondary', length: 'medium' },
  { id: 'tertiary', length: 'xlong' },
  { id: 'quaternary', length: 'long' },
]

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
      <TextSkeleton length="medium" size="heading" />
      <TextSkeleton className="mt-3" length="long" />
      <div className="mt-8 divide-y divide-border border-y border-border">
        {DEFAULT_ROWS.map((row) => (
          <div className="py-5" key={row.id}>
            <TextSkeleton length={row.length} />
            <TextSkeleton className="mt-2" length="medium" size="meta" />
          </div>
        ))}
      </div>
    </div>
  )
}
