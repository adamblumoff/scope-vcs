import { Skeleton } from '@/components/ui/skeleton'

export function RunGraphSkeleton() {
  return (
    <div className="mt-3 overflow-x-auto border-y border-border p-4">
      <div className="flex min-w-[656px] items-center gap-12">
        {[0, 1, 2].map((node) => (
          <div className="flex items-center gap-12" key={node}>
            {node > 0 ? <Skeleton className="h-0.5 w-10" /> : null}
            <div className="h-[84px] w-[208px] shrink-0 border border-border p-4">
              <Skeleton className="h-4 w-28" />
              <Skeleton className="mt-3 h-3 w-20" />
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
