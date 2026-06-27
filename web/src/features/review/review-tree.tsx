import type { ProjectionPreviewAudience, ReviewFile, Visibility } from '@/api/types'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
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
import { useMemo, useState } from 'react'
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

const EDITABLE_REVIEW_TREE_COLUMNS =
  'sm:grid-cols-[minmax(0,1fr)_110px_120px_120px]'
const READONLY_REVIEW_TREE_COLUMNS =
  'sm:grid-cols-[minmax(0,1fr)_110px_120px]'
const COMPACT_READONLY_REVIEW_TREE_COLUMNS =
  'sm:grid-cols-[minmax(0,1fr)_84px_28px]'
const PUBLIC_TREE_COLUMNS = 'sm:grid-cols-[minmax(0,1fr)]'
const VISIBILITY_ACTION_CLASS = 'w-[88px] justify-start'
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
            <div className={compactVisibility ? 'text-center' : undefined}>
              {compactVisibility ? (
                <span className="sr-only">Visibility</span>
              ) : (
                'Visibility'
              )}
            </div>
            {editable && <div className="text-right">Set</div>}
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

  if (node.type === 'file') {
    const nextVisibility = node.file.visibility === 'Public' ? 'Private' : 'Public'
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
          selected &&
            'bg-brand-muted shadow-[inset_2px_0_0_0_var(--brand)]',
          viewAudience === 'public' && !visibleInView && 'text-muted-foreground',
          columnsClassName,
        )}
      >
        {onSelectFile ? (
          <button
            className="flex min-w-0 items-center gap-2 rounded-md text-left transition-colors hover:bg-muted/70"
            onClick={() => onSelectFile(node.file)}
            style={{ paddingLeft: `${depth * 18}px` }}
            type="button"
          >
            <FilePathLabel path={node.path} />
          </button>
        ) : (
          <div
            className="flex min-w-0 items-center gap-2"
            style={{ paddingLeft: `${depth * 18}px` }}
          >
            <FilePathLabel path={node.path} />
          </div>
        )}
        {!publicTree && (
          <>
            <div className="flex flex-wrap gap-1.5 text-xs leading-4">
              {viewAudience && <ViewState visible={visibleInView} />}
              {stagedReview && (
                <Badge variant="outline">
                  {'kind' in node.file ? node.file.kind : 'Modified'}
                </Badge>
              )}
            </div>
            <div className={cn(compactVisibility && 'flex justify-center')}>
              <VisibilityBadge
                compact={compactVisibility}
                visibility={node.file.visibility}
              />
            </div>
            {editable && onSetVisibility ? (
              <div className="flex justify-end">
                <Button
                  aria-label={`Set ${displayPath(node.path)} ${nextVisibility.toLowerCase()}`}
                  className={VISIBILITY_ACTION_CLASS}
                  disabled={disabled || busy || pendingKey !== null}
                  onClick={() =>
                    onSetVisibility([node.file], nextVisibility, node.key)
                  }
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  {busy ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : nextVisibility === 'Public' ? (
                    <Globe2 className="size-3.5" />
                  ) : (
                    <Lock className="size-3.5" />
                  )}
                  <span>{nextVisibility}</span>
                </Button>
              </div>
            ) : null}
          </>
        )}
      </div>
    )
  }

  const isCollapsed = collapsed.has(node.key)
  const visibility = folderVisibility(node.files)
  const nextVisibility = visibility === 'Public' ? 'Private' : 'Public'
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
            <div className={cn(compactVisibility && 'flex justify-center')}>
              <VisibilityBadge
                compact={compactVisibility}
                visibility={visibility}
              />
            </div>
            {editable && onSetVisibility ? (
              <div className="flex justify-end">
                <Button
                  aria-label={`Set ${node.path} ${nextVisibility.toLowerCase()}`}
                  className={VISIBILITY_ACTION_CLASS}
                  disabled={disabled || busy || pendingKey !== null}
                  onClick={() =>
                    onSetVisibility(node.files, nextVisibility, node.key)
                  }
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  {busy ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : nextVisibility === 'Public' ? (
                    <Globe2 className="size-3.5" />
                  ) : (
                    <Lock className="size-3.5" />
                  )}
                  <span>{nextVisibility}</span>
                </Button>
              </div>
            ) : null}
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
