import type {
  ProjectionPreviewAudience,
  ReviewFile,
  Visibility,
  VisibilityState,
} from '@/api/types'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import {
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderOpen,
  Globe2,
  LoaderCircle,
  Lock,
} from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { audienceLabel } from './review-labels'
import {
  buildReviewTree,
  displayPath,
  folderCollapseKeys,
  folderVisibility,
  type ReviewTreeNode,
  visibleFileCountInProjectionPreview,
  visibleInProjectionPreview,
} from './review-tree-model'

const EDITABLE_REVIEW_TREE_COLUMNS = 'sm:grid-cols-[minmax(0,1fr)_140px]'
const READONLY_REVIEW_TREE_COLUMNS =
  'sm:grid-cols-[minmax(0,1fr)_110px_120px]'
const COMPACT_READONLY_REVIEW_TREE_COLUMNS =
  'sm:grid-cols-[minmax(0,1fr)_84px_28px]'
const PUBLIC_TREE_COLUMNS = 'sm:grid-cols-[minmax(0,1fr)]'
export type ReviewTreeVariant = 'workflow' | 'public'

export function ReviewTree({
  audience,
  compactVisibility = false,
  disabled = false,
  files,
  onSetVisibility,
  onSelectFile,
  pendingKey = null,
  selectedFilePath = null,
  visiblePaths,
  variant = 'workflow',
  stagedReview,
}: {
  audience?: ProjectionPreviewAudience
  compactVisibility?: boolean
  disabled?: boolean
  files: ReviewFile[]
  onSetVisibility?: (
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) => void
  onSelectFile?: (file: ReviewFile) => void
  pendingKey?: string | null
  selectedFilePath?: string | null
  visiblePaths?: ReadonlySet<string>
  variant?: ReviewTreeVariant
  stagedReview: boolean
}) {
  const root = useMemo(() => buildReviewTree(files), [files])
  const editable = Boolean(onSetVisibility)
  const publicTree = variant === 'public'
  const columnsClassName = treeColumns({
    compactVisibility,
    editable,
    publicTree,
  })
  const [collapsed, setCollapsed] = useState<Set<string>>(
    () => new Set(folderCollapseKeys(root)),
  )

  function toggleFolder(key: string) {
    setCollapsed((current) => {
      const next = new Set(current)
      if (next.has(key)) {
        next.delete(key)
      } else {
        next.add(key)
      }
      return next
    })
  }

  return (
    <TooltipProvider>
      <div className="divide-y divide-border">
        <div
          className={cn(
            'hidden gap-3 px-2 py-2 text-xs font-medium leading-4 text-muted-foreground sm:grid',
            columnsClassName,
          )}
        >
          <div>Path</div>
          {!publicTree && (
            <>
              <div>
                {audience
                  ? `${audienceLabel(audience)} view`
                  : stagedReview
                    ? 'Change'
                    : 'Scope'}
              </div>
              {!editable && (
                <div className={compactVisibility ? 'text-center' : undefined}>
                  {compactVisibility ? (
                    <span className="sr-only">Visibility</span>
                  ) : (
                    'Visibility'
                  )}
                </div>
              )}
            </>
          )}
        </div>
        {root.children.map((node) => (
          <ReviewTreeNodeRow
            collapsed={collapsed}
            columnsClassName={columnsClassName}
            compactVisibility={compactVisibility}
            depth={0}
            disabled={disabled}
            editable={editable}
            key={node.key}
            node={node}
            onSelectFile={onSelectFile}
            onSetVisibility={onSetVisibility}
            onToggleFolder={toggleFolder}
            pendingKey={pendingKey}
            selectedFilePath={selectedFilePath}
            stagedReview={stagedReview}
            viewAudience={audience}
            visiblePaths={visiblePaths}
            variant={variant}
          />
        ))}
      </div>
    </TooltipProvider>
  )
}
function ReviewTreeNodeRow({
  collapsed,
  columnsClassName,
  compactVisibility,
  depth,
  disabled,
  editable,
  node,
  onSelectFile,
  onSetVisibility,
  onToggleFolder,
  pendingKey,
  selectedFilePath,
  stagedReview,
  viewAudience,
  visiblePaths,
  variant,
}: {
  collapsed: Set<string>
  columnsClassName: string
  compactVisibility: boolean
  depth: number
  disabled: boolean
  editable: boolean
  node: ReviewTreeNode
  onSelectFile?: (file: ReviewFile) => void
  onSetVisibility?: (
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) => void
  onToggleFolder: (key: string) => void
  pendingKey: string | null
  selectedFilePath: string | null
  stagedReview: boolean
  viewAudience?: ProjectionPreviewAudience
  visiblePaths?: ReadonlySet<string>
  variant: ReviewTreeVariant
}) {
  const publicTree = variant === 'public'
  const showToggle = editable && Boolean(onSetVisibility)

  if (node.type === 'file') {
    const busy = pendingKey === node.key
    const visibleInView = visibleInProjectionPreview(
      node.path,
      viewAudience,
      visiblePaths,
    )
    const selected =
      selectedFilePath !== null &&
      displayPath(selectedFilePath) === displayPath(node.file.path)
    return (
      <div
        className={cn(
          'grid gap-2 px-2 py-2.5 text-sm sm:items-center',
          selected && 'bg-brand-muted shadow-[inset_2px_0_0_0_var(--brand)]',
          viewAudience === 'public' && !visibleInView && 'text-muted-foreground',
          columnsClassName,
        )}
      >
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          {showToggle && onSetVisibility && (
            <VisibilityToggle
              busy={busy}
              disabled={disabled || pendingKey !== null}
              onSelect={(visibility) =>
                onSetVisibility([node.file], visibility, node.key)
              }
              targetLabel={displayPath(node.path)}
              visibility={node.file.visibility}
            />
          )}
          {onSelectFile ? (
            <button
              className="flex min-w-0 flex-1 items-center gap-2 rounded-md text-left transition-colors hover:bg-muted/70"
              onClick={() => onSelectFile(node.file)}
              type="button"
            >
              <FilePathLabel path={node.path} />
            </button>
          ) : (
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <FilePathLabel path={node.path} />
            </div>
          )}
        </div>
        {!publicTree && (
          <>
            <div className="flex flex-wrap items-center gap-1.5 text-xs leading-4">
              {viewAudience && <ViewState visible={visibleInView} />}
              {stagedReview && (
                <Badge variant="neutral">
                  {'kind' in node.file ? node.file.kind : 'Modified'}
                </Badge>
              )}
            </div>
            {!editable && (
              <div className={cn(compactVisibility && 'flex justify-center')}>
                <VisibilityBadge
                  compact={compactVisibility}
                  visibility={node.file.visibility}
                />
              </div>
            )}
          </>
        )}
      </div>
    )
  }

  const isCollapsed = collapsed.has(node.key)
  const visibility = folderVisibility(node.files)
  const busy = pendingKey === node.key
  const viewState = folderViewState(node.files, viewAudience, visiblePaths)

  return (
    <>
      <div
        className={cn(
          'grid gap-2 bg-muted/20 px-2 py-2.5 text-sm sm:items-center',
          columnsClassName,
        )}
      >
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          {showToggle && onSetVisibility && (
            <VisibilityToggle
              busy={busy}
              disabled={disabled || pendingKey !== null}
              onSelect={(nextVisibility) =>
                onSetVisibility(node.files, nextVisibility, node.key)
              }
              targetLabel={node.path}
              visibility={visibility}
            />
          )}
          <Button
            aria-label={`${isCollapsed ? 'Expand' : 'Collapse'} ${node.name}`}
            onClick={() => onToggleFolder(node.key)}
            size="icon-xs"
            type="button"
            variant="secondary"
          >
            {isCollapsed ? (
              <ChevronRight className="size-3" />
            ) : (
              <ChevronDown className="size-3" />
            )}
          </Button>
          {isCollapsed ? (
            <Folder className="size-4 shrink-0 text-muted-foreground" />
          ) : (
            <FolderOpen className="size-4 shrink-0 text-muted-foreground" />
          )}
          <span className="min-w-0 truncate font-mono text-xs" title={node.path}>
            {node.name}
          </span>
        </div>
        {!publicTree && (
          <>
            <div className="text-xs leading-4 text-muted-foreground">
              {viewAudience ? (
                <ViewState
                  partialLabel={
                    viewState.visibleCount > 0 &&
                    viewState.visibleCount < viewState.totalCount
                      ? `${viewState.visibleCount}/${viewState.totalCount} shown`
                      : undefined
                  }
                  visible={viewState.visibleCount > 0}
                />
              ) : (
                <>
                  {node.files.length}{' '}
                  {node.files.length === 1 ? 'file' : 'files'}
                </>
              )}
            </div>
            {!editable && (
              <div className={cn(compactVisibility && 'flex justify-center')}>
                <VisibilityBadge
                  compact={compactVisibility}
                  visibility={visibility}
                />
              </div>
            )}
          </>
        )}
      </div>
      {!isCollapsed &&
        node.children.map((child) => (
          <ReviewTreeNodeRow
            collapsed={collapsed}
            columnsClassName={columnsClassName}
            compactVisibility={compactVisibility}
            depth={depth + 1}
            disabled={disabled}
            editable={editable}
            key={child.key}
            node={child}
            onSelectFile={onSelectFile}
            onSetVisibility={onSetVisibility}
            onToggleFolder={onToggleFolder}
            pendingKey={pendingKey}
            selectedFilePath={selectedFilePath}
            stagedReview={stagedReview}
            viewAudience={viewAudience}
            visiblePaths={visiblePaths}
            variant={variant}
          />
        ))}
    </>
  )
}

function VisibilityToggle({
  busy,
  disabled,
  onSelect,
  targetLabel,
  visibility,
}: {
  busy: boolean
  disabled: boolean
  onSelect: (visibility: Visibility) => void
  targetLabel: string
  visibility: Visibility | VisibilityState
}) {
  const isPublic = visibility === 'Public'
  const isPrivate = visibility === 'Private'

  return (
    <fieldset
      aria-label={
        visibility === 'Mixed'
          ? `${targetLabel} visibility: mixed`
          : `${targetLabel} visibility: ${visibility.toLowerCase()}`
      }
      className="inline-flex shrink-0 items-center rounded-md border border-border bg-muted p-0.5"
    >
      {busy ? (
        <span className="flex h-6 w-[3.25rem] items-center justify-center">
          <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" />
        </span>
      ) : (
        <>
          <VisibilitySegment
            active={isPublic}
            description="Public — kept in the public projection"
            disabled={disabled}
            icon={<Globe2 className="size-3.5" />}
            label={`Make ${targetLabel} public`}
            onSelect={() => onSelect('Public')}
            tone="public"
          />
          <VisibilitySegment
            active={isPrivate}
            description="Private — hidden from the public projection"
            disabled={disabled}
            icon={<Lock className="size-3.5" />}
            label={`Make ${targetLabel} private`}
            onSelect={() => onSelect('Private')}
            tone="private"
          />
        </>
      )}
    </fieldset>
  )
}

function VisibilitySegment({
  active,
  description,
  disabled,
  icon,
  label,
  onSelect,
  tone,
}: {
  active: boolean
  description: string
  disabled: boolean
  icon: ReactNode
  label: string
  onSelect: () => void
  tone: 'private' | 'public'
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={label}
          aria-pressed={active}
          className={cn(
            'flex size-6 items-center justify-center rounded-[5px] text-muted-foreground/70 transition-colors',
            !active && !disabled && 'hover:bg-background/70 hover:text-foreground',
            active &&
              tone === 'public' &&
              'bg-green-100 text-green-900 shadow-[var(--shadow-card)] dark:bg-green-500/20 dark:text-green-300',
            active &&
              tone === 'private' &&
              'bg-red-100 text-red-900 shadow-[var(--shadow-card)] dark:bg-red-500/20 dark:text-red-300',
            active && 'cursor-default',
            disabled && 'pointer-events-none opacity-50',
          )}
          disabled={disabled}
          onClick={() => {
            if (!active) {
              onSelect()
            }
          }}
          type="button"
        >
          {icon}
        </button>
      </TooltipTrigger>
      <TooltipContent>{description}</TooltipContent>
    </Tooltip>
  )
}

function treeColumns({
  compactVisibility,
  editable,
  publicTree,
}: {
  compactVisibility: boolean
  editable: boolean
  publicTree: boolean
}) {
  if (publicTree) {
    return PUBLIC_TREE_COLUMNS
  }
  if (editable) {
    return EDITABLE_REVIEW_TREE_COLUMNS
  }
  return compactVisibility
    ? COMPACT_READONLY_REVIEW_TREE_COLUMNS
    : READONLY_REVIEW_TREE_COLUMNS
}

function FilePathLabel({ path }: { path: string }) {
  return (
    <>
      <span className="size-6 shrink-0" />
      <File className="size-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 truncate font-mono text-xs" title={path}>
        {displayPath(path)}
      </span>
    </>
  )
}

function folderViewState(
  files: ReviewFile[],
  audience: ProjectionPreviewAudience | undefined,
  visiblePaths: ReadonlySet<string> | undefined,
) {
  const totalCount = files.length
  const visibleCount = visibleFileCountInProjectionPreview(
    files,
    audience,
    visiblePaths,
  )
  return { totalCount, visibleCount }
}

function ViewState({
  partialLabel,
  visible,
}: {
  partialLabel?: string
  visible: boolean
}) {
  return (
    <Badge variant="outline">
      {partialLabel ?? (visible ? 'Shown' : 'Hidden')}
    </Badge>
  )
}
