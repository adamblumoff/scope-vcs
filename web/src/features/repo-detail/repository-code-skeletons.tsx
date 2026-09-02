import {
  BlockSkeleton,
  LineSkeleton,
  TextSkeleton,
  type LineSkeletonLength,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'

const PENDING_FILES: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'long' },
  { id: 'third', length: 'short' },
  { id: 'fourth', length: 'long' },
  { id: 'fifth', length: 'medium' },
  { id: 'sixth', length: 'long' },
]

const PENDING_SOURCE_LINES: { id: string; length: LineSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
  { id: 'fifth', length: 'short' },
  { id: 'sixth', length: 'long' },
  { id: 'seventh', length: 'medium' },
  { id: 'eighth', length: 'long' },
  { id: 'ninth', length: 'short' },
]

export function FileNavigatorSkeleton() {
  return (
    <div className="divide-y divide-border">
      {PENDING_FILES.map((file) => (
        <div
          className="grid min-h-9 grid-cols-[18px_minmax(0,1fr)_64px] items-center gap-2"
          key={file.id}
        >
          <BlockSkeleton className="size-3.5" />
          <TextSkeleton length={file.length} size="meta" />
          <TextSkeleton length="short" size="meta" />
        </div>
      ))}
    </div>
  )
}

export function SourceCodeSkeleton() {
  return (
    <div className="space-y-3 p-5 sm:p-7">
      {PENDING_SOURCE_LINES.map((line) => (
        <div className="flex items-center gap-4" key={line.id}>
          <TextSkeleton length="tiny" size="meta" />
          <LineSkeleton length={line.length} />
        </div>
      ))}
    </div>
  )
}
