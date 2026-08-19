import { ApplicationPendingShell } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

export function AccountPagePending() {
  return (
    <ApplicationPendingShell contextLabel="Account" label="Loading account">
      <div className="py-8 lg:py-10">
        <h1 className="text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[32px]">
          Account
        </h1>
        <p className="mt-2 text-[15px] leading-6 text-muted-foreground">
          Manage Scope CLI access for this account.
        </p>
        <div className="mt-6 divide-y divide-border">
          {[0, 1].map((row) => (
            <section
              className="grid gap-4 py-5 md:grid-cols-[240px_minmax(0,1fr)]"
              key={row}
            >
              <div>
                <Skeleton className="h-4 w-32" />
                <Skeleton className="mt-2 h-3 w-48 max-w-full" />
                <Skeleton className="mt-1.5 h-3 w-40 max-w-4/5" />
              </div>
              <div className="md:pt-0.5">
                <Skeleton className="h-9 w-32" />
                {row === 1 ? <Skeleton className="mt-4 h-14 w-full" /> : null}
              </div>
            </section>
          ))}
        </div>
      </div>
    </ApplicationPendingShell>
  )
}
