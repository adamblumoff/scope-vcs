import {
  BlockSkeleton,
  LineSkeleton,
  TextSkeleton,
  type LineSkeletonLength,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'

const COMMIT_FILES: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'long' },
  { id: 'third', length: 'medium' },
  { id: 'fourth', length: 'long' },
  { id: 'fifth', length: 'medium' },
]
const DIFF_LINES: { id: string; length: LineSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'short' },
  { id: 'fifth', length: 'long' },
  { id: 'sixth', length: 'medium' },
]

export function CommitDetailSkeleton({ showDiff }: { showDiff: boolean }) {
  return (
    <div className="min-w-0">
      <div className="border-b border-border px-5 py-4 sm:px-6">
        <TextSkeleton length="long" />
        <TextSkeleton className="mt-2" length="medium" size="meta" />
      </div>
      <div className="grid min-w-0 grid-cols-1 xl:grid-cols-[minmax(0,0.9fr)_minmax(360px,1.1fr)]">
        <div className="min-w-0 divide-y divide-border">
          {COMMIT_FILES.map((file) => (
            <div
              className="flex min-h-9 min-w-0 items-center gap-3 px-5"
              key={file.id}
            >
              <BlockSkeleton className="size-3.5" />
              <div className="min-w-0 flex-1">
                <TextSkeleton length={file.length} size="meta" />
              </div>
              <BlockSkeleton className="ml-auto h-5 w-16 rounded-full" />
            </div>
          ))}
        </div>
        <div className="h-[70vh] min-h-[340px] max-h-[720px] min-w-0 border-border p-5 xl:border-l">
          {showDiff ? (
            <div className="space-y-3">
              {DIFF_LINES.map((line) => (
                <LineSkeleton key={line.id} length={line.length} />
              ))}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  )
}
