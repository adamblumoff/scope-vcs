import { ApplicationPendingShell } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

const REPOSITORY_NAME_LENGTHS = [24, 18, 30, 21]

export function OwnerProfilePending({ owner }: { owner: string }) {
  return (
    <ApplicationPendingShell label={`Loading @${owner}`}>
      <div className="py-8 lg:py-10">
        <h1 className="text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[32px]">
          @{owner}
        </h1>
        <div className="mt-6 divide-y divide-border border-y border-border">
          {REPOSITORY_NAME_LENGTHS.map((length) => (
            <div className="py-4" key={length}>
              <Skeleton className="h-5" style={{ width: `${length}ch` }} />
              <Skeleton className="mt-2 h-3 w-36" />
            </div>
          ))}
        </div>
      </div>
    </ApplicationPendingShell>
  )
}
