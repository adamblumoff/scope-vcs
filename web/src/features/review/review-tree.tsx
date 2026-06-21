import type { ReviewFile, Visibility } from '@/api/types'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Check,
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderOpen,
  LoaderCircle,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  buildReviewTree,
  displayPath,
  folderCollapseKeys,
  folderVisibility,
  type ReviewTreeNode,
} from './review-tree-model'

export function ReviewTree({
  disabled,
  files,
  onSetVisibility,
  pendingKey,
  stagedReview,
}: {
  disabled: boolean
  files: ReviewFile[]
  onSetVisibility: (
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) => void
  pendingKey: string | null
  stagedReview: boolean
}) {
  const root = useMemo(() => buildReviewTree(files), [files])
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
      <div className="hidden grid-cols-[minmax(0,1fr)_110px_120px_120px] gap-3 px-2 py-2 text-xs font-medium leading-4 text-muted-foreground sm:grid">
        <div>Path</div>
        <div>{stagedReview ? 'Change' : 'Scope'}</div>
        <div>Visibility</div>
        <div className="text-right">Set</div>
      </div>
      {root.children.map((node) => (
        <ReviewTreeNodeRow
          collapsed={collapsed}
          depth={0}
          disabled={disabled}
          key={node.key}
          node={node}
          onSetVisibility={onSetVisibility}
          onToggleFolder={toggleFolder}
          pendingKey={pendingKey}
          stagedReview={stagedReview}
        />
      ))}
    </div>
  )
}

function ReviewTreeNodeRow({
  collapsed,
  depth,
  disabled,
  node,
  onSetVisibility,
  onToggleFolder,
  pendingKey,
  stagedReview,
}: {
  collapsed: Set<string>
  depth: number
  disabled: boolean
  node: ReviewTreeNode
  onSetVisibility: (
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) => void
  onToggleFolder: (key: string) => void
  pendingKey: string | null
  stagedReview: boolean
}) {
  if (node.type === 'file') {
    const nextVisibility = node.file.visibility === 'Public' ? 'Private' : 'Public'
    const busy = pendingKey === node.key
    return (
      <div className="grid gap-2 px-2 py-2.5 text-sm sm:grid-cols-[minmax(0,1fr)_110px_120px_120px] sm:items-center">
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          <span className="size-6 shrink-0" />
          <File className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 truncate font-mono text-xs" title={node.path}>
            {displayPath(node.path)}
          </span>
        </div>
        <div>
          {stagedReview && (
            <Badge variant="outline">
              {'kind' in node.file ? node.file.kind : 'Modified'}
            </Badge>
          )}
        </div>
        <div>
          <VisibilityBadge visibility={node.file.visibility} />
        </div>
        <div className="flex justify-end">
          <Button
            aria-label={`Set ${displayPath(node.path)} ${nextVisibility.toLowerCase()}`}
            disabled={disabled || busy || pendingKey !== null}
            onClick={() => onSetVisibility([node.file], nextVisibility, node.key)}
            size="sm"
            type="button"
            variant="secondary"
          >
            {busy ? (
              <LoaderCircle className="size-3.5 animate-spin" />
            ) : (
              <Check className="size-3.5" />
            )}
            <span>{nextVisibility}</span>
          </Button>
        </div>
      </div>
    )
  }

  const isCollapsed = collapsed.has(node.key)
  const visibility = folderVisibility(node.files)
  const nextVisibility = visibility === 'Public' ? 'Private' : 'Public'
  const busy = pendingKey === node.key

  return (
    <>
      <div className="grid gap-2 bg-muted/20 px-2 py-2.5 text-sm sm:grid-cols-[minmax(0,1fr)_110px_120px_120px] sm:items-center">
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
        <div className="text-xs leading-4 text-muted-foreground">
          {node.files.length} {node.files.length === 1 ? 'file' : 'files'}
        </div>
        <div>
          <VisibilityBadge visibility={visibility} />
        </div>
        <div className="flex justify-end">
          <Button
            aria-label={`Set ${node.path} ${nextVisibility.toLowerCase()}`}
            disabled={disabled || busy || pendingKey !== null}
            onClick={() => onSetVisibility(node.files, nextVisibility, node.key)}
            size="sm"
            type="button"
            variant="secondary"
          >
            {busy ? (
              <LoaderCircle className="size-3.5 animate-spin" />
            ) : (
              <Check className="size-3.5" />
            )}
            <span>{nextVisibility}</span>
          </Button>
        </div>
      </div>
      {!isCollapsed &&
        node.children.map((child) => (
          <ReviewTreeNodeRow
            collapsed={collapsed}
            depth={depth + 1}
            disabled={disabled}
            key={child.key}
            node={child}
            onSetVisibility={onSetVisibility}
            onToggleFolder={onToggleFolder}
            pendingKey={pendingKey}
            stagedReview={stagedReview}
          />
        ))}
    </>
  )
}
