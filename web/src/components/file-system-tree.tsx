import type { ReactNode } from 'react'
import { useMemo, useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { VisibilityBadge } from '@/components/visibility-badge'
import { cn } from '@/lib/utils'
import {
  ChevronDown,
  ChevronRight,
  File,
  FileSearch,
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
const META_ONLY_TREE_COLUMNS = 'sm:grid-cols-[minmax(0,1fr)_110px]'
const VISIBILITY_ONLY_TREE_COLUMNS = 'sm:grid-cols-[minmax(0,1fr)_120px]'
const PATH_ONLY_TREE_COLUMNS = 'sm:grid-cols-[minmax(0,1fr)]'

export function FileSystemTreePanel<TFile extends FileSystemTreeFileBase>({
  description,
  emptyDescription,
  emptyTitle,
  files,
  getFileMeta,
  getFolderMeta,
  metaColumnLabel,
  title = 'Files',
}: {
  description?: string
  emptyDescription: string
  emptyTitle: string
  files: TFile[]
  getFileMeta?: (file: TFile) => ReactNode
  getFolderMeta?: (files: TFile[]) => ReactNode
  metaColumnLabel?: ReactNode
  title?: string
}) {
  return (
    <section className="mt-8">
      <div className="flex flex-col gap-3 border-b border-border py-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-base font-semibold leading-6 text-balance">
              {title}
            </h2>
            <Badge variant="neutral">{files.length}</Badge>
          </div>
          {description && (
            <p className="mt-1 max-w-[720px] text-sm leading-5 text-muted-foreground text-pretty">
              {description}
            </p>
          )}
        </div>
      </div>

      <div className="border-b border-border py-4">
        {files.length === 0 ? (
          <FileSystemTreeEmpty
            description={emptyDescription}
            title={emptyTitle}
          />
        ) : (
          <FileSystemTree
            files={files}
            getFileMeta={getFileMeta}
            getFolderMeta={getFolderMeta}
            metaColumnLabel={metaColumnLabel}
          />
        )}
      </div>
    </section>
  )
}

export function FileSystemTree<TFile extends FileSystemTreeFileBase>({
  compactVisibility = false,
  files,
  getFileMeta,
  getFolderMeta = defaultFolderMeta,
  metaColumnLabel = 'Status',
  onSelectFile,
  selectedFilePath = null,
  showMeta = true,
  showVisibility = true,
}: {
  compactVisibility?: boolean
  files: TFile[]
  getFileMeta?: (file: TFile) => ReactNode
  getFolderMeta?: (files: TFile[]) => ReactNode
  metaColumnLabel?: ReactNode
  onSelectFile?: (file: TFile) => void
  selectedFilePath?: string | null
  showMeta?: boolean
  showVisibility?: boolean
}) {
  const root = useMemo(() => buildFileSystemTree(files), [files])
  const treeKey = useMemo(
    () => files.map((file) => displayPath(file.path)).join('\0'),
    [files],
  )
  const columnsClassName = treeColumns({
    compactVisibility,
    showMeta,
    showVisibility,
  })

  return (
    <FileSystemTreeRows
      columnsClassName={columnsClassName}
      compactVisibility={compactVisibility}
      getFileMeta={getFileMeta}
      getFolderMeta={getFolderMeta}
      key={treeKey}
      onSelectFile={onSelectFile}
      root={root}
      selectedFilePath={selectedFilePath}
      showMeta={showMeta}
      showVisibility={showVisibility}
      metaColumnLabel={metaColumnLabel}
    />
  )
}

function FileSystemTreeRows<TFile extends FileSystemTreeFileBase>({
  columnsClassName,
  compactVisibility,
  getFileMeta,
  getFolderMeta,
  metaColumnLabel,
  onSelectFile,
  root,
  selectedFilePath,
  showMeta,
  showVisibility,
}: {
  columnsClassName: string
  compactVisibility: boolean
  getFileMeta?: (file: TFile) => ReactNode
  getFolderMeta: (files: TFile[]) => ReactNode
  metaColumnLabel: ReactNode
  onSelectFile?: (file: TFile) => void
  root: Extract<FileSystemTreeNode<TFile>, { type: 'folder' }>
  selectedFilePath: string | null
  showMeta: boolean
  showVisibility: boolean
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
    <div className="divide-y divide-border">
      <div
        className={cn(
          'hidden gap-3 px-2 py-2 text-xs font-medium leading-4 text-muted-foreground sm:grid',
          columnsClassName,
        )}
      >
        <div>Path</div>
        {showMeta && <div>{metaColumnLabel}</div>}
        {showVisibility && (
          <div className={compactVisibility ? 'text-center' : undefined}>
            {compactVisibility ? (
              <span className="sr-only">Visibility</span>
            ) : (
              'Visibility'
            )}
          </div>
        )}
      </div>
      {root.children.map((node) => (
        <FileSystemTreeNodeRow
          collapsed={collapsed}
          columnsClassName={columnsClassName}
          compactVisibility={compactVisibility}
          depth={0}
          getFileMeta={getFileMeta}
          getFolderMeta={getFolderMeta}
          key={node.key}
          node={node}
          onSelectFile={onSelectFile}
          onToggleFolder={toggleFolder}
          selectedFilePath={selectedFilePath}
          showMeta={showMeta}
          showVisibility={showVisibility}
        />
      ))}
    </div>
  )
}

function FileSystemTreeNodeRow<TFile extends FileSystemTreeFileBase>({
  collapsed,
  columnsClassName,
  compactVisibility,
  depth,
  getFileMeta,
  getFolderMeta,
  node,
  onSelectFile,
  onToggleFolder,
  selectedFilePath,
  showMeta,
  showVisibility,
}: {
  collapsed: Set<string>
  columnsClassName: string
  compactVisibility: boolean
  depth: number
  getFileMeta?: (file: TFile) => ReactNode
  getFolderMeta: (files: TFile[]) => ReactNode
  node: FileSystemTreeNode<TFile>
  onSelectFile?: (file: TFile) => void
  onToggleFolder: (key: string) => void
  selectedFilePath: string | null
  showMeta: boolean
  showVisibility: boolean
}) {
  if (node.type === 'file') {
    const selected =
      selectedFilePath !== null &&
      displayPath(selectedFilePath) === displayPath(node.file.path)
    return (
      <div
        className={cn(
          'grid gap-2 px-2 py-2.5 text-sm sm:items-center',
          selected && 'bg-brand-muted shadow-[inset_2px_0_0_0_var(--brand)]',
          columnsClassName,
        )}
      >
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
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
        {showMeta && (
          <div className="flex flex-wrap items-center gap-1.5 text-xs leading-4">
            {getFileMeta?.(node.file)}
          </div>
        )}
        {showVisibility && (
          <div className={cn(compactVisibility && 'flex justify-center')}>
            <VisibilityBadge
              compact={compactVisibility}
              visibility={node.file.visibility}
            />
          </div>
        )}
      </div>
    )
  }

  const isCollapsed = collapsed.has(node.key)
  const visibility = folderVisibility(node.files)

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
        {showMeta && (
          <div className="text-xs leading-4 text-muted-foreground">
            {getFolderMeta(node.files)}
          </div>
        )}
        {showVisibility && (
          <div className={cn(compactVisibility && 'flex justify-center')}>
            <VisibilityBadge compact={compactVisibility} visibility={visibility} />
          </div>
        )}
      </div>
      {!isCollapsed &&
        node.children.map((child) => (
          <FileSystemTreeNodeRow
            collapsed={collapsed}
            columnsClassName={columnsClassName}
            compactVisibility={compactVisibility}
            depth={depth + 1}
            getFileMeta={getFileMeta}
            getFolderMeta={getFolderMeta}
            key={child.key}
            node={child}
            onSelectFile={onSelectFile}
            onToggleFolder={onToggleFolder}
            selectedFilePath={selectedFilePath}
            showMeta={showMeta}
            showVisibility={showVisibility}
          />
        ))}
    </>
  )
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

function FileSystemTreeEmpty({
  description,
  title,
}: {
  description: string
  title: string
}) {
  return (
    <div className="flex items-center gap-3.5 py-8">
      <div className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-brand-muted text-brand">
        <FileSearch className="size-5" />
      </div>
      <div className="min-w-0 text-sm">
        <div className="text-base font-semibold leading-6 text-balance">
          {title}
        </div>
        <div className="mt-0.5 leading-5 text-muted-foreground text-pretty">
          {description}
        </div>
      </div>
    </div>
  )
}

function defaultFolderMeta(files: FileSystemTreeFileBase[]) {
  return (
    <>
      {files.length} {files.length === 1 ? 'file' : 'files'}
    </>
  )
}

function treeColumns({
  compactVisibility,
  showMeta,
  showVisibility,
}: {
  compactVisibility: boolean
  showMeta: boolean
  showVisibility: boolean
}) {
  if (showMeta && showVisibility) {
    return compactVisibility ? COMPACT_TREE_COLUMNS : FULL_TREE_COLUMNS
  }
  if (showMeta) {
    return META_ONLY_TREE_COLUMNS
  }
  if (showVisibility) {
    return VISIBILITY_ONLY_TREE_COLUMNS
  }
  return PATH_ONLY_TREE_COLUMNS
}
