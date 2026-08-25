import type { ReviewFileDiff } from '@/api/types'
import { PanelState } from '@/components/empty-state'
import { displayPath } from '@/components/file-system-tree-model'
import { PendingSurface } from '@/components/pending-surface'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { parseDiffFromFile, type FileDiffMetadata } from '@pierre/diffs'
import { FileDiff } from '@pierre/diffs/react'
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
                ? null
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

        <div
          className="min-h-0 flex-1 overflow-auto"
          onScroll={(event) => onScrollTopChange?.(event.currentTarget.scrollTop)}
          ref={scrollRef}
        >
          {loading ? (
            <PendingSurface
              className="min-h-full"
              delay
              label={`Loading ${displayName || 'file'} diff`}
            >
              <DiffSkeleton />
            </PendingSurface>
          ) : error ? (
            <PanelState role="alert" tone="error">
              <TriangleAlert className="size-5" />
              <span>{error}</span>
              {onRetry && (
                <Button onClick={onRetry} size="sm" type="button" variant="secondary">
                  Retry
                </Button>
              )}
            </PanelState>
          ) : contentSides.binary.length > 0 && contentSides.text.length > 0 ? (
            <MixedContentDiffState
              binary={contentSides.binary}
              text={contentSides.text}
            />
          ) : contentSides.binary.length > 0 ? (
            <BinaryDiffState sides={contentSides.binary} />
          ) : fileDiff && fileDiff.hunks.length > 0 ? (
            <div className="review-diff-viewer scope-content-enter">
              <FileDiff
                disableWorkerPool={typeof Worker === 'undefined'}
                fileDiff={fileDiff}
                options={diffOptions}
              />
            </div>
          ) : (
            <PanelState>
              <FileText className="size-5" />
              <span>{emptyDiffLabel(diff)}</span>
            </PanelState>
          )}
        </div>
      </div>
    </aside>
  )
}

const DIFF_SKELETON_WIDTHS = [78, 46, 86, 62, 72, 38, 82, 56, 68]

function DiffSkeleton() {
  return (
    <div className="py-3 font-mono">
      {DIFF_SKELETON_WIDTHS.map((width, index) => (
        <div
          className={cn(
            'grid min-h-7 grid-cols-[36px_minmax(0,1fr)] items-center gap-3 px-4',
            index === 3 || index === 4 ? 'bg-success-soft/50' : undefined,
          )}
          key={`${width}-${index}`}
        >
          <Skeleton className="h-3 w-5" />
          <Skeleton className="h-3" style={{ width: `${width}%` }} />
        </div>
      ))}
    </div>
  )
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
    <PanelState>
      <BinarySummary sides={sides} />
    </PanelState>
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
