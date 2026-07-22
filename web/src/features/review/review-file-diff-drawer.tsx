import type { ReviewFileDiff } from '@/api/types'
import { displayPath } from '@/components/file-system-tree-model'
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
import { File, FileText, TriangleAlert, X } from 'lucide-react'
import { type ReactNode, useLayoutEffect, useMemo, useRef } from 'react'
import {
  type BinaryContentSide,
  type ReviewFileContent,
  reviewContentSides,
  type TextContentSide,
} from './review-file-content'
import { parsedDiffForReviewFile } from './review-file-diff-cache'

const PIERRE_DIFF_OPTIONS = {
  diffStyle: 'unified',
  disableFileHeader: true,
  hunkSeparators: 'line-info-basic',
  lineDiffType: 'word',
  overflow: 'wrap',
} as const

const PIERRE_WORKER_HIGHLIGHTER_OPTIONS = {} satisfies WorkerInitializationRenderOptions

export function ReviewFileDiffDrawer({
  cacheKey,
  className,
  diff,
  error,
  loading,
  onClose,
  onRetry,
  onScrollTopChange,
  scrollTop = 0,
  selectedPath,
  showHeader = true,
}: {
  cacheKey?: string | null
  className?: string
  diff: ReviewFileDiff | null
  error: string | null
  loading: boolean
  onClose?: () => void
  onRetry?: () => void
  onScrollTopChange?: (scrollTop: number) => void
  scrollTop?: number
  selectedPath: string | null
  showHeader?: boolean
}) {
  const themeType = useThemeType()
  const fileDiff = useMemo(
    () =>
      diff
        ? parsedDiffForReviewFile(diff, cacheKey, diffMetadataForReviewFile)
        : null,
    [cacheKey, diff],
  )
  const contentSides = useMemo(
    () => (diff ? reviewContentSides(diff) : { binary: [], text: [] }),
    [diff],
  )
  const diffOptions = useMemo(
    () => ({ ...PIERRE_DIFF_OPTIONS, themeType }),
    [themeType],
  )
  const workerPoolOptions = useMemo(createPierreWorkerPoolOptions, [])
  const displayName = displayPath(diff?.path ?? selectedPath ?? '')
  const scrollRef = useRef<HTMLDivElement>(null)
  const restoredScrollKeyRef = useRef<string | null>(null)
  const scrollKey = cacheKey ?? selectedPath

  useLayoutEffect(() => {
    if (restoredScrollKeyRef.current === scrollKey) return
    restoredScrollKeyRef.current = scrollKey
    if (scrollRef.current) scrollRef.current.scrollTop = scrollTop
  })

  return (
    <aside
      aria-label={displayName ? `${displayName} diff` : 'File diff'}
      className={cn('h-full min-h-[340px] bg-background', className)}
    >
      <div className="flex h-full min-h-0 flex-col">
        {showHeader ? (
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
                {loading
                  ? 'Loading diff…'
                  : error
                    ? 'Diff unavailable'
                    : modeChangeLabel(diff) ?? 'Diff'}
              </div>
            </div>
            {onClose && (
              <Button
                aria-label="Close diff viewer"
                onClick={onClose}
                size="icon-xs"
                type="button"
                variant="ghost"
              >
                <X className="size-3.5" />
              </Button>
            )}
          </div>
        ) : null}

        <div
          className="min-h-0 flex-1 overflow-auto"
          onScroll={(event) => onScrollTopChange?.(event.currentTarget.scrollTop)}
          ref={scrollRef}
        >
          {loading ? (
            <DiffSkeleton />
          ) : error ? (
            <DiffState role="alert" tone="error">
              <TriangleAlert className="size-4 text-destructive" />
              <span>{error}</span>
              {onRetry && (
                <Button onClick={onRetry} size="sm" type="button" variant="secondary">
                  Retry
                </Button>
              )}
            </DiffState>
          ) : contentSides.binary.length > 0 && contentSides.text.length > 0 ? (
            <MixedContentDiffState
              binary={contentSides.binary}
              text={contentSides.text}
            />
          ) : contentSides.binary.length > 0 ? (
            <BinaryDiffState sides={contentSides.binary} />
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
  if (oldText === null || newText === null) return null

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

function textContents(content: ReviewFileContent | null) {
  if (!content) return ''
  return content.kind === 'text' ? content.text : null
}

function BinaryDiffState({ sides }: { sides: BinaryContentSide[] }) {
  return (
    <div className="flex min-h-[220px] items-center justify-center px-4 py-6 text-sm text-muted-foreground">
      <BinarySummary sides={sides} />
    </div>
  )
}

function MixedContentDiffState({
  binary,
  text,
}: {
  binary: BinaryContentSide[]
  text: TextContentSide[]
}) {
  return (
    <div className="min-h-[220px]">
      <div className="border-b border-border px-4 py-4 text-sm text-muted-foreground">
        <BinarySummary sides={binary} />
      </div>
      {text.map((side) => (
        <section key={side.label}>
          <div className="border-b border-border px-4 py-2 text-xs font-medium text-muted-foreground">
            {side.label} text
          </div>
          <pre className="overflow-auto whitespace-pre-wrap break-words px-4 py-3 font-mono text-xs leading-5 text-foreground">
            {side.text || 'Empty text file'}
          </pre>
        </section>
      ))}
    </div>
  )
}

function BinarySummary({ sides }: { sides: BinaryContentSide[] }) {
  return (
    <div className="w-full max-w-md space-y-3">
      <div className="flex items-center gap-2 text-foreground">
        <File className="size-4" />
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
  const modeChange = modeChangeLabel(diff)
  if (modeChange) {
    return modeChange
  }
  if (diff?.kind === 'Added') {
    return 'Empty file added'
  }
  if (diff?.kind === 'Deleted') {
    return 'Empty file deleted'
  }
  return 'No content changes'
}

function modeChangeLabel(diff: ReviewFileDiff | null) {
  if (!diff?.old_mode || !diff.new_mode || diff.old_mode === diff.new_mode) {
    return null
  }
  return `Mode ${diff.old_mode} → ${diff.new_mode}`
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
  role,
  tone = 'muted',
}: {
  children: ReactNode
  role?: 'alert'
  tone?: 'error' | 'muted'
}) {
  return (
    <div
      className={cn(
        'flex min-h-[220px] items-center justify-center gap-2 px-4 text-sm leading-5',
        tone === 'error' ? 'text-destructive' : 'text-muted-foreground',
      )}
      role={role}
    >
      {children}
    </div>
  )
}
