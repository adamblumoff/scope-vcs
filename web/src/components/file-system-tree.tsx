import type { ReactNode } from 'react'
import { useMemo, useState } from 'react'
import { Button } from '@/components/ui/button'
import { VisibilityBadge } from '@/components/visibility-badge'
import { cn } from '@/lib/utils'
import {
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderOpen,
} from 'lucide-react'
import {
  buildFileSystemTree,
  displayPath,
  folderCollapseKeys,
  folderVisibility,
  type FileSystemTreeFileBase,
  type FileSystemTreeNode,
} from './file-system-tree-model'

const FULL_TREE_COLUMNS = 'sm:grid-cols-[minmax(0,1fr)_110px_120px]'
const COMPACT_TREE_COLUMNS = 'sm:grid-cols-[minmax(0,1fr)_84px_28px]'

export function FileSystemTree<TFile extends FileSystemTreeFileBase>({
  compactVisibility = false,
  files,
  getFileMeta,
  metaColumnLabel = 'Status',
  onSelectFile,
  selectedFilePath = null,
}: {
  compactVisibility?: boolean
  files: TFile[]
  getFileMeta?: (file: TFile) => ReactNode
  metaColumnLabel?: ReactNode
  onSelectFile?: (file: TFile) => void
  selectedFilePath?: string | null
}) {
  const root = useMemo(() => buildFileSystemTree(files), [files])
  const treeKey = useMemo(
    () => files.map((file) => displayPath(file.path)).join('\0'),
    [files],
  )
  const columnsClassName = compactVisibility
    ? COMPACT_TREE_COLUMNS
    : FULL_TREE_COLUMNS

  return (
    <FileSystemTreeRows
      columnsClassName={columnsClassName}
      compactVisibility={compactVisibility}
      getFileMeta={getFileMeta}
      key={treeKey}
      onSelectFile={onSelectFile}
      root={root}
      selectedFilePath={selectedFilePath}
      metaColumnLabel={metaColumnLabel}
    />
  )
}

function FileSystemTreeRows<TFile extends FileSystemTreeFileBase>({
  columnsClassName,
  compactVisibility,
  getFileMeta,
  metaColumnLabel,
  onSelectFile,
  root,
  selectedFilePath,
}: {
  columnsClassName: string
  compactVisibility: boolean
  getFileMeta?: (file: TFile) => ReactNode
  metaColumnLabel: ReactNode
  onSelectFile?: (file: TFile) => void
  root: Extract<FileSystemTreeNode<TFile>, { type: 'folder' }>
  selectedFilePath: string | null
}) {
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
    <div>
      <div
        className={cn(
          'hidden min-h-10 gap-3 px-3 py-2 font-mono text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground sm:grid sm:items-center',
          columnsClassName,
        )}
      >
        <div>Path</div>
        <div>{metaColumnLabel}</div>
        <div className={compactVisibility ? 'text-center' : undefined}>
          {compactVisibility ? (
            <span className="sr-only">Visibility</span>
          ) : (
            'Visibility'
          )}
        </div>
      </div>
      <ul className="space-y-1">
        {root.children.map((node) => (
          <FileSystemTreeNodeRow
            collapsed={collapsed}
            columnsClassName={columnsClassName}
            compactVisibility={compactVisibility}
            depth={0}
            getFileMeta={getFileMeta}
            key={node.key}
            node={node}
            onSelectFile={onSelectFile}
            onToggleFolder={toggleFolder}
            selectedFilePath={selectedFilePath}
          />
        ))}
      </ul>
    </div>
  )
}

function FileSystemTreeNodeRow<TFile extends FileSystemTreeFileBase>({
  collapsed,
  columnsClassName,
  compactVisibility,
  depth,
  getFileMeta,
  node,
  onSelectFile,
  onToggleFolder,
  selectedFilePath,
}: {
  collapsed: Set<string>
  columnsClassName: string
  compactVisibility: boolean
  depth: number
  getFileMeta?: (file: TFile) => ReactNode
  node: FileSystemTreeNode<TFile>
  onSelectFile?: (file: TFile) => void
  onToggleFolder: (key: string) => void
  selectedFilePath: string | null
}) {
  if (node.type === 'file') {
    const selected =
      selectedFilePath !== null &&
      displayPath(selectedFilePath) === displayPath(node.file.path)
    return (
      <li
        className={cn(
          'relative grid min-h-12 gap-2 rounded-md border border-transparent px-3 py-2.5 text-sm transition-[background-color,border-color] hover:bg-muted/55 sm:items-center',
          selected &&
            'border-[var(--border-strong)] bg-muted shadow-[inset_2px_0_0_0_var(--platinum-bright)] hover:bg-muted',
          columnsClassName,
        )}
      >
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          {onSelectFile ? (
            <button
              aria-current={selected ? 'true' : undefined}
              className="flex min-w-0 flex-1 items-center gap-2 rounded text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
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
        <div className="flex flex-wrap items-center gap-1.5 text-xs leading-4">
          <span className="font-medium text-muted-foreground sm:hidden">
            Status
          </span>
          {getFileMeta?.(node.file)}
        </div>
        <div
          className={cn(
            'flex items-center gap-1.5',
            compactVisibility && 'sm:justify-center',
          )}
        >
          <span className="text-xs font-medium text-muted-foreground sm:hidden">
            Visibility
          </span>
          <VisibilityBadge
            compact={compactVisibility}
            visibility={node.file.visibility}
          />
        </div>
      </li>
    )
  }

  const isCollapsed = collapsed.has(node.key)
  const visibility = folderVisibility(node.files)

  return (
    <>
      <li
        className={cn(
          'grid min-h-12 gap-2 rounded-md border border-transparent px-3 py-2.5 text-sm transition-colors hover:bg-muted/40 sm:items-center',
          columnsClassName,
        )}
      >
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          <Button
            aria-expanded={!isCollapsed}
            aria-label={`${isCollapsed ? 'Expand' : 'Collapse'} ${node.name}`}
            onClick={() => onToggleFolder(node.key)}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            {isCollapsed ? (
              <ChevronRight className="size-3" />
            ) : (
              <ChevronDown className="size-3" />
            )}
          </Button>
          {isCollapsed ? (
            <Folder className="size-4 shrink-0 text-[var(--platinum)]" strokeWidth={1.7} />
          ) : (
            <FolderOpen className="size-4 shrink-0 text-[var(--platinum)]" strokeWidth={1.7} />
          )}
          <span className="min-w-0 truncate font-mono text-xs" title={node.path}>
            {node.name}
          </span>
        </div>
        <div className="flex items-center gap-1.5 text-xs leading-4 text-muted-foreground">
          <span className="font-medium sm:hidden">Status</span>
          {node.files.length} {node.files.length === 1 ? 'file' : 'files'}
        </div>
        <div
          className={cn(
            'flex items-center gap-1.5',
            compactVisibility && 'sm:justify-center',
          )}
        >
          <span className="text-xs font-medium text-muted-foreground sm:hidden">
            Visibility
          </span>
          <VisibilityBadge compact={compactVisibility} visibility={visibility} />
        </div>
      </li>
      {!isCollapsed &&
        node.children.map((child) => (
          <FileSystemTreeNodeRow
            collapsed={collapsed}
            columnsClassName={columnsClassName}
            compactVisibility={compactVisibility}
            depth={depth + 1}
            getFileMeta={getFileMeta}
            key={child.key}
            node={child}
            onSelectFile={onSelectFile}
            onToggleFolder={onToggleFolder}
            selectedFilePath={selectedFilePath}
          />
        ))}
    </>
  )
}

function FilePathLabel({ path }: { path: string }) {
  return (
    <>
      <span className="size-6 shrink-0" />
      <File className="size-4 shrink-0 text-[var(--platinum)]" strokeWidth={1.7} />
      <span className="min-w-0 truncate font-mono text-xs" title={path}>
        {displayPath(path)}
      </span>
    </>
  )
}
