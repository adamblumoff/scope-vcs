import type { ReviewFileDiff } from '@/api/types'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { parseDiffFromFile, type FileDiffMetadata } from '@pierre/diffs'
import {
  FileDiff,
  WorkerPoolContextProvider,
  type WorkerInitializationRenderOptions,
  type WorkerPoolOptions,
} from '@pierre/diffs/react'
import { Skeleton } from '@/components/ui/skeleton'
import { useThemeType } from '@/lib/use-theme-type'
import { FileText, TriangleAlert, X } from 'lucide-react'
import { type ReactNode, useMemo } from 'react'
import { displayPath } from './review-tree-model'

const PIERRE_DIFF_OPTIONS = {
  diffStyle: 'unified',
  disableFileHeader: true,
  hunkSeparators: 'line-info-basic',
  lineDiffType: 'word',
  overflow: 'wrap',
} as const

const PIERRE_WORKER_HIGHLIGHTER_OPTIONS = {} satisfies WorkerInitializationRenderOptions

export function ReviewFileDiffDrawer({
  className,
  diff,
  error,
  loading,
  onClose,
  selectedPath,
}: {
  className?: string
  diff: ReviewFileDiff | null
  error: string | null
  loading: boolean
  onClose: () => void
  selectedPath: string | null
}) {
  const themeType = useThemeType()
  const fileDiff = useMemo(
    () => (diff ? diffMetadataForReviewFile(diff) : null),
    [diff],
  )
  const binarySides = useMemo(
    () => (diff ? binaryContentSides(diff) : []),
    [diff],
  )
  const diffOptions = useMemo(
    () => ({ ...PIERRE_DIFF_OPTIONS, themeType }),
    [themeType],
  )
  const workerPoolOptions = useMemo(createPierreWorkerPoolOptions, [])
  const displayName = displayPath(diff?.path ?? selectedPath ?? '')

  return (
    <aside
      aria-label={displayName ? `${displayName} diff` : 'File diff'}
      className={cn('h-full min-h-[340px] bg-background', className)}
    >
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex min-h-14 items-center gap-3 border-b border-border px-3 py-2.5">
          <FileText className="size-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <div
              className="truncate font-mono text-xs font-medium leading-5"
              title={displayName}
            >
              {displayName || 'Diff'}
            </div>
            <div className="text-xs leading-4 text-muted-foreground">
              {loading ? 'Loading diff' : error ? 'Diff unavailable' : 'Diff'}
            </div>
          </div>
          <Button
            aria-label="Close diff viewer"
            onClick={onClose}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <X className="size-3.5" />
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-auto">
          {loading ? (
            <DiffSkeleton />
          ) : error ? (
            <DiffState tone="error">
              <TriangleAlert className="size-4 text-destructive" />
              <span>{error}</span>
            </DiffState>
          ) : binarySides.length > 0 ? (
            <BinaryDiffState sides={binarySides} />
          ) : fileDiff && fileDiff.hunks.length > 0 ? (
            <div className="review-diff-viewer">
              <PierreFileDiff
                fileDiff={fileDiff}
                options={diffOptions}
                workerPoolOptions={workerPoolOptions}
              />
            </div>
          ) : (
            <DiffState>
              <FileText className="size-4 text-muted-foreground" />
              <span>{emptyDiffLabel(diff)}</span>
            </DiffState>
          )}
        </div>
      </div>
    </aside>
  )
}

function PierreFileDiff({
  fileDiff,
  options,
  workerPoolOptions,
}: {
  fileDiff: FileDiffMetadata
  options: typeof PIERRE_DIFF_OPTIONS & { themeType: 'dark' | 'light' }
  workerPoolOptions: WorkerPoolOptions | null
}) {
  const renderer = (
    <FileDiff
      disableWorkerPool={!workerPoolOptions}
      fileDiff={fileDiff}
      options={options}
    />
  )

  if (!workerPoolOptions) {
    return renderer
  }

  return (
    <WorkerPoolContextProvider
      highlighterOptions={PIERRE_WORKER_HIGHLIGHTER_OPTIONS}
      poolOptions={workerPoolOptions}
    >
      {renderer}
    </WorkerPoolContextProvider>
  )
}

function createPierreWorkerPoolOptions(): WorkerPoolOptions | null {
  if (typeof Worker === 'undefined') {
    return null
  }

  return {
    poolSize: pierreWorkerPoolSize(),
    workerFactory: () =>
      new Worker(
        new URL('@pierre/diffs/worker/worker-portable.js', import.meta.url),
        { type: 'module' },
      ),
  }
}

function pierreWorkerPoolSize() {
  if (typeof navigator === 'undefined' || !navigator.hardwareConcurrency) {
    return 2
  }
  return Math.min(4, Math.max(1, navigator.hardwareConcurrency))
}

function diffMetadataForReviewFile(diff: ReviewFileDiff): FileDiffMetadata | null {
  const oldText = textContents(diff.old_content)
  const newText = textContents(diff.new_content)
  if (oldText === null || newText === null) {
    return null
  }

  return parseDiffFromFile(
    {
      contents: oldText,
      name: diff.path,
    },
    {
      contents: newText,
      name: diff.path,
    },
  )
}

type ReviewFileContent = NonNullable<ReviewFileDiff['old_content']>

type BinaryContentSide = {
  label: string
  oid: string
  sizeBytes: number
}

function textContents(content: ReviewFileContent | null) {
  if (!content) {
    return ''
  }
  return content.kind === 'text' ? content.text : null
}

function binaryContentSides(diff: ReviewFileDiff): BinaryContentSide[] {
  return [
    binarySide('Old', diff.old_content),
    binarySide('New', diff.new_content),
  ].filter((side): side is BinaryContentSide => Boolean(side))
}

function binarySide(
  label: string,
  content: ReviewFileContent | null,
): BinaryContentSide | null {
  if (!content || content.kind !== 'binary') {
    return null
  }
  return {
    label,
    oid: content.oid,
    sizeBytes: content.size_bytes,
  }
}

function BinaryDiffState({ sides }: { sides: BinaryContentSide[] }) {
  return (
    <div className="flex min-h-[220px] items-center justify-center px-4 py-6 text-sm text-muted-foreground">
      <div className="w-full max-w-md space-y-3">
        <div className="flex items-center gap-2 text-foreground">
          <FileText className="size-4" />
          <span className="font-medium">Binary file not rendered</span>
        </div>
        <div className="space-y-2 font-mono text-xs leading-5">
          {sides.map((side) => (
            <div
              className="grid grid-cols-[44px_minmax(0,1fr)] gap-x-3"
              key={`${side.label}-${side.oid}`}
            >
              <span className="text-muted-foreground">{side.label}</span>
              <span className="min-w-0 break-all">
                {formatBytes(side.sizeBytes)} - {abbreviateOid(side.oid)}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

function abbreviateOid(oid: string) {
  return oid.length > 12 ? oid.slice(0, 12) : oid
}

function formatBytes(bytes: number) {
  if (bytes < 1024) {
    return `${bytes} B`
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function emptyDiffLabel(diff: ReviewFileDiff | null) {
  if (diff?.kind === 'Added') {
    return 'Empty file added'
  }
  if (diff?.kind === 'Deleted') {
    return 'Empty file deleted'
  }
  return 'No content changes'
}

const DIFF_SKELETON_WIDTHS = [
  'w-[82%]',
  'w-[64%]',
  'w-[91%]',
  'w-[48%]',
  'w-[73%]',
  'w-[86%]',
  'w-[57%]',
  'w-[78%]',
  'w-[40%]',
  'w-[69%]',
]

function DiffSkeleton() {
  return (
    <div className="space-y-2.5 px-4 py-4 font-mono">
      {DIFF_SKELETON_WIDTHS.map((width) => (
        <Skeleton className={cn('h-3.5', width)} key={width} />
      ))}
    </div>
  )
}

function DiffState({
  children,
  tone = 'muted',
}: {
  children: ReactNode
  tone?: 'error' | 'muted'
}) {
  return (
    <div
      className={cn(
        'flex min-h-[220px] items-center justify-center gap-2 px-4 text-sm leading-5',
        tone === 'error' ? 'text-destructive' : 'text-muted-foreground',
      )}
    >
      {children}
    </div>
  )
}
