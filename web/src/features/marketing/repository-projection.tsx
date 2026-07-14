import type { Visibility, VisibilityState } from '@/api/types'
import {
  buildFileSystemTree,
  folderVisibility,
  type FileSystemTreeNode,
} from '@/components/file-system-tree-model'
import { VisibilityBadge } from '@/components/visibility-badge'
import {
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderGit2,
  FolderOpen,
} from 'lucide-react'
import { useState, type ReactElement } from 'react'

type ProjectionAudience = 'private' | 'public'

type ProjectionFile = {
  path: string
  visibility: Visibility
}

type ProjectionRow = {
  active?: boolean
  depth: number
  expanded?: boolean
  key: string
  name: string
  path: string
  type: 'file' | 'folder'
  visibility: VisibilityState
}

type ProjectionViewDefinition = {
  audience: ProjectionAudience
  label: string
  rows: readonly ProjectionRow[]
}

type ProjectionViewProps = ProjectionViewDefinition & {
  hoveredPath: string | null
  onHoverRow: (row: ProjectionRow | null) => void
}

const repositoryFiles: readonly ProjectionFile[] = [
  { path: 'src/cli/index.ts', visibility: 'Public' },
  { path: 'src/internal/policy.ts', visibility: 'Private' },
  { path: 'src/shared/config.ts', visibility: 'Public' },
  { path: '.env', visibility: 'Private' },
  { path: 'README.md', visibility: 'Public' },
]

const activeFolderPaths: Record<ProjectionAudience, string> = {
  private: '/src/internal',
  public: '/src/cli',
}

const expandedFolderPaths = new Set(['/src', '/src/cli'])

const treeRowMetrics = {
  chevronSize: 12,
  disclosureSlotSize: 24,
  fileIconSize: 16,
  itemGap: 8,
  levelIndent: 16,
  rowInset: 8,
} as const
const fileIconInset = (
  treeRowMetrics.disclosureSlotSize - treeRowMetrics.chevronSize
) / 2
const fileLabelGap = treeRowMetrics.disclosureSlotSize
  + treeRowMetrics.itemGap
  - fileIconInset
  - treeRowMetrics.fileIconSize

const projectionViews = [
  {
    audience: 'public',
    label: 'Public view',
    rows: buildProjectionRows(
      repositoryFiles.filter((file) => file.visibility === 'Public'),
      'public',
    ),
  },
  {
    audience: 'private',
    label: 'Private view',
    rows: buildProjectionRows(repositoryFiles, 'private'),
  },
] as const satisfies ReadonlyArray<ProjectionViewDefinition>

export function RepositoryProjection(): ReactElement {
  const [hoveredRow, setHoveredRow] = useState<ProjectionRow | null>(null)
  const sourceContext = hoveredRow ? projectionSourcePath(hoveredRow) : null

  return (
    <section
      aria-labelledby="repository-views-title"
      className="marketing-projection pointer-events-none absolute inset-0"
      data-private-only={hoveredRow?.visibility === 'Private' || undefined}
      id="repository-views"
    >
      <h2 className="sr-only" id="repository-views-title">
        One repository projected into public and private views
      </h2>

      <div
        className="marketing-source-node"
        data-hover-visibility={hoveredRow?.visibility}
        data-projection-node="repository"
        data-source-context={sourceContext ?? undefined}
        id="repository-source"
      >
        <span aria-hidden className="marketing-source-icon">
          <FolderGit2 />
        </span>
        <span className="marketing-source-copy">
          <strong>scope/</strong>
          {sourceContext && <span title={sourceContext}>{sourceContext}</span>}
        </span>
        <span className="marketing-source-branch">main</span>
        <span aria-hidden className="marketing-source-junction" />
      </div>

      <ProjectionConnections />

      {projectionViews.map((view) => (
        <ProjectionView
          hoveredPath={hoveredRow?.path ?? null}
          key={view.audience}
          onHoverRow={setHoveredRow}
          {...view}
        />
      ))}
    </section>
  )
}

function ProjectionConnections(): ReactElement {
  return (
    <svg
      aria-hidden
      className="marketing-connections"
      preserveAspectRatio="none"
      viewBox="0 0 100 100"
    >
      <path
        className="marketing-connection marketing-connection-public marketing-connection-stacked"
        d="M 50 9 C 50 10, 50 10.5, 50 11.5"
      />
      <path
        className="marketing-connection marketing-connection-private marketing-connection-stacked"
        d="M 50 9 C 66 16, 66 35, 55 48.5"
      />
      <path
        className="marketing-connection marketing-connection-public marketing-connection-desktop"
        d="M 71 50 C 73 50, 69 28, 73 28"
      />
      <path
        className="marketing-connection marketing-connection-private marketing-connection-desktop"
        d="M 71 50 C 73 50, 69 80, 73 80"
      />
    </svg>
  )
}

function ProjectionView({
  audience,
  hoveredPath,
  label,
  onHoverRow,
  rows,
}: ProjectionViewProps): ReactElement {
  return (
    <article
      className={`marketing-view marketing-view-${audience}`}
      data-projection-node={audience}
    >
      <header className="marketing-view-header">
        <h3>{label}</h3>
      </header>
      <ul className="px-2 py-2">
        {rows.map((row) => (
          <li key={row.key}>
            <div
              className="marketing-file-row pointer-events-auto w-full text-left"
              data-active={row.active}
              data-highlighted={hoveredPath === row.path || undefined}
              data-path={row.path}
              onPointerEnter={() => onHoverRow(row)}
              onPointerLeave={() => onHoverRow(null)}
              style={{ paddingLeft: projectionRowInset(row) }}
            >
              <span
                className="flex min-w-0 items-center"
                style={{ gap: row.type === 'file' ? fileLabelGap : treeRowMetrics.itemGap }}
              >
                {row.type === 'folder' && (
                  <span
                    aria-hidden
                    className="grid size-6 shrink-0 place-items-center text-[var(--platinum)]"
                  >
                    <ProjectionDisclosureIcon expanded={row.expanded} />
                  </span>
                )}
                <ProjectionFileIcon expanded={row.expanded} type={row.type} />
                <span className="min-w-0 truncate font-mono text-xs">{row.name}</span>
              </span>
              <VisibilityBadge compact visibility={row.visibility} />
            </div>
          </li>
        ))}
      </ul>
    </article>
  )
}

function buildProjectionRows(
  files: readonly ProjectionFile[],
  audience: ProjectionAudience,
): ProjectionRow[] {
  const tree = buildFileSystemTree([...files])
  return flattenProjectionTree(tree.children, activeFolderPaths[audience])
}

function flattenProjectionTree(
  nodes: FileSystemTreeNode<ProjectionFile>[],
  activePath: string,
  depth = 0,
): ProjectionRow[] {
  return nodes.flatMap((node) => {
    if (node.type === 'file') {
      return [{
        depth,
        key: node.key,
        name: node.name,
        path: node.path,
        type: node.type,
        visibility: node.file.visibility,
      }]
    }

    const expanded = expandedFolderPaths.has(node.path)
    const row: ProjectionRow = {
      active: node.path === activePath || undefined,
      depth,
      expanded,
      key: node.key,
      name: `${node.name}/`,
      path: node.path,
      type: node.type,
      visibility: folderVisibility(node.files),
    }

    return expanded
      ? [row, ...flattenProjectionTree(node.children, activePath, depth + 1)]
      : [row]
  })
}

function projectionRowInset(row: ProjectionRow): number {
  const depthInset = treeRowMetrics.rowInset
    + row.depth * treeRowMetrics.levelIndent

  // A file icon replaces the disclosure slot: its edge aligns with a
  // centered chevron, while the derived gap keeps labels aligned with folders.
  return row.type === 'file' ? depthInset + fileIconInset : depthInset
}

function projectionSourcePath(row: ProjectionRow): string {
  const path = row.path.replace(/^\//, '')
  return row.type === 'folder' ? `${path}/` : path
}

function ProjectionDisclosureIcon({
  expanded,
}: Pick<ProjectionRow, 'expanded'>): ReactElement {
  if (expanded) return <ChevronDown className="size-3" />
  return <ChevronRight className="size-3" />
}

function ProjectionFileIcon({
  expanded,
  type,
}: Pick<ProjectionRow, 'expanded' | 'type'>): ReactElement {
  const className = 'size-4 shrink-0 text-[var(--platinum)]'

  if (type === 'file') {
    return <File aria-hidden className={className} strokeWidth={1.7} />
  }

  if (expanded) {
    return <FolderOpen aria-hidden className={className} strokeWidth={1.7} />
  }

  return <Folder aria-hidden className={className} strokeWidth={1.7} />
}
