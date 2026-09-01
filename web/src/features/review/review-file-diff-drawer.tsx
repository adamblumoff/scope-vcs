import type {
  ReviewDiffBinarySide,
  ReviewDiffTextSide,
  ReviewFileDiff,
} from '@/api/types'
import { PanelState } from '@/components/empty-state'
import { displayPath } from '@/components/file-system-tree-model'
import { PendingSurface } from '@/components/pending-surface'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import DOMPurify from 'dompurify'
import { File, FileText, TriangleAlert, X } from 'lucide-react'
import { type ReactNode, useLayoutEffect, useRef } from 'react'
import {
  reviewFileDiffEmptyLabel,
  reviewFileDiffModeChangeLabel,
  reviewFileDiffOmittedLabel,
} from './review-file-diff-presentation'

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
  const displayName = displayPath(diff?.path ?? selectedPath ?? '')
  const scrollRef = useRef<HTMLDivElement>(null)
  const restoredScrollKeyRef = useRef<string | null>(null)
  const scrollKey = cacheKey ?? selectedPath ?? null

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
                  : reviewFileDiffModeChangeLabel(diff) ?? 'Diff'}
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
          ) : diff?.presentation.kind === 'mixed' ? (
            <MixedContentDiffState
              binary={diff.presentation.binary}
              text={diff.presentation.text}
            />
          ) : diff?.presentation.kind === 'binary' ? (
            <BinaryDiffState sides={diff.presentation.sides} />
          ) : diff?.presentation.kind === 'html' ? (
            <PrerenderedDiff html={diff.presentation.html} />
          ) : diff?.presentation.kind === 'omitted' ? (
            <OmittedDiffState reason={diff.presentation.reason} />
          ) : (
            <PanelState>
              <FileText className="size-5" />
              <span>{reviewFileDiffEmptyLabel(diff)}</span>
            </PanelState>
          )}
        </div>
      </div>
    </aside>
  )
}

function PrerenderedDiff({ html }: { html: string }) {
  const containerRef = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    const container = containerRef.current
    if (!container) return

    const fragment = DOMPurify.sanitize(html, {
      RETURN_DOM_FRAGMENT: true,
    })
    const host = document.createElement('diffs-container')
    host.attachShadow({ mode: 'open' }).replaceChildren(fragment)
    container.replaceChildren(host)
  }, [html])

  return (
    <div
      className="review-diff-viewer scope-content-enter"
      ref={containerRef}
      suppressHydrationWarning
    />
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

function BinaryDiffState({ sides }: { sides: ReviewDiffBinarySide[] }) {
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
  binary: ReviewDiffBinarySide[]
  text: ReviewDiffTextSide[]
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
            {side.content || 'Empty text file'}
          </pre>
          {side.truncated && (
            <div className="border-t border-border px-4 py-2 text-xs text-muted-foreground">
              Text preview truncated
            </div>
          )}
        </section>
      ))}
    </div>
  )
}

function BinarySummary({ sides }: { sides: ReviewDiffBinarySide[] }) {
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

function OmittedDiffState({
  reason,
}: {
  reason: Extract<ReviewFileDiff['presentation'], { kind: 'omitted' }>['reason']
}) {
  return (
    <PanelState>
      <FileText className="size-5" />
      <span>{reviewFileDiffOmittedLabel(reason)}</span>
    </PanelState>
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
