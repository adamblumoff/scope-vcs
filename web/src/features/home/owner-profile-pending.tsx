import { ApplicationPendingShell } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

export function OwnerProfilePending({ owner }: { owner: string }) {
  return (
    <ApplicationPendingShell label={`Loading @${owner}`}>
      <div className="py-8 lg:py-10">
        <h1 className="text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[32px]">
          @{owner}
        </h1>
        <div className="mt-6 divide-y divide-border border-y border-border">
          {[72, 58, 84, 66].map((width) => (
            <div className="py-4" key={width}>
              <Skeleton className="h-5" style={{ width: `${width}%` }} />
              <Skeleton className="mt-2 h-3 w-36 max-w-1/2" />
            </div>
          ))}
        </div>
      </div>
    </ApplicationPendingShell>
  )
}
