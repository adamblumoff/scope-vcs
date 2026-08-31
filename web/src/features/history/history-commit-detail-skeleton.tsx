import { Skeleton } from '@/components/ui/skeleton'

const COMMIT_FILE_SKELETON_WIDTHS = [58, 72, 46, 66, 52]

export function CommitDetailSkeleton({ showDiff }: { showDiff: boolean }) {
  return (
    <div className="min-w-0">
      <div className="border-b border-border px-5 py-4 sm:px-6">
        <Skeleton className="h-4 w-2/5" />
        <Skeleton className="mt-2 h-3 w-40" />
      </div>
      <div className="grid xl:grid-cols-[minmax(0,0.9fr)_minmax(360px,1.1fr)]">
        <div className="divide-y divide-border">
          {COMMIT_FILE_SKELETON_WIDTHS.map((width) => (
            <div className="flex min-h-9 items-center gap-3 px-5" key={width}>
              <Skeleton className="size-3.5" />
              <Skeleton className="h-3" style={{ width: `${width}%` }} />
              <Skeleton className="ml-auto h-5 w-16 rounded-full" />
            </div>
          ))}
        </div>
        <div className="h-[70vh] min-h-[340px] max-h-[720px] border-border p-5 xl:border-l">
          {showDiff ? (
            <div className="space-y-3">
              {[82, 56, 74, 44, 88, 64].map((width) => (
                <Skeleton className="h-3" key={width} style={{ width: `${width}%` }} />
              ))}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  )
}
