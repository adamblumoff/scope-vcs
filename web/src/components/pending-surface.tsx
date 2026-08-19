import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageRail } from '@/components/page-header'
import { cn } from '@/lib/utils'

export function PendingSurface({
  className,
  label = 'Loading page',
}: {
  className?: string
  label?: string
}) {
  return (
    <output
      aria-busy="true"
      className={cn('block min-h-full w-full', className)}
    >
      <span className="sr-only">{label}</span>
    </output>
  )
}

export function ApplicationPendingShell({
  contextLabel,
  label,
  repository,
}: {
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
        <PendingSurface label={label} />
      </PageRail>
    </AppShell>
  )
}
