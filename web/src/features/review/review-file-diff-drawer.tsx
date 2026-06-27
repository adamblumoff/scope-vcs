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

function diffMetadataForReviewFile(diff: ReviewFileDiff): FileDiffMetadata {
  return parseDiffFromFile(
    {
      contents: diff.old_content ?? '',
      name: diff.path,
    },
    {
      contents: diff.new_content ?? '',
      name: diff.path,
    },
  )
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
      {DIFF_SKELETON_WIDTHS.map((width, index) => (
        <Skeleton className={cn('h-3.5', width)} key={index} />
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
