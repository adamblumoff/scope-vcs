import type {
  RepoContent,
  RepoFile,
  RepoFileContent,
  RepoParams,
} from '@/api/types'
import { EmptyState, PanelState } from '@/components/empty-state'
import { FileSystemTree } from '@/components/file-system-tree'
import { PendingSurface } from '@/components/pending-surface'
import { isRepositoryHtmlPath } from '@/components/repository-html'
import { RepositoryHtmlRenderer } from '@/components/repository-html-renderer'
import { isRepositoryMarkdownPath } from '@/components/repository-markdown'
import { RepositoryMarkdownRenderer } from '@/components/repository-markdown-renderer'
import { Button } from '@/components/ui/button'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import { VisibilityBadge } from '@/components/visibility-badge'
import { WorkspaceTabStrip } from '@/components/workspace-tab-strip'
import {
  workspaceTabDomIds,
  workspaceTabPanelId,
  pruneWorkspaceTabs,
  type WorkspaceTabItem,
} from '@/components/workspace-tab-model'
import {
  displayRouteFilePath,
  selectedRouteFilePath,
} from '@/lib/route-file'
import { FileQuestion, TriangleAlert } from 'lucide-react'
import {
  useLayoutEffect,
  useMemo,
  useRef,
  type ReactNode,
} from 'react'
import {
  readRepositorySourceScroll,
  writeRepositorySourceScroll,
} from './repository-source-scroll-cache'
import {
  FileNavigatorSkeleton,
  SourceCodeSkeleton,
} from './repository-code-skeletons'

const CODE_TAB_SET_ID = 'repository-code-files'

export function RepositoryCodeView({
  content,
  contentError,
  contentRetry,
  onSelectFilePath,
  params,
  selectedFile,
  selectedFileError,
  selectedFileIdentity,
  selectedFileLoading,
  selectedFileRetry,
  selectedPath,
}: {
  content: RepoContent | null
  contentError: string | null
  contentRetry: () => void
  onSelectFilePath: (path: string) => void
  params: RepoParams
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedFileIdentity: string | null
  selectedFileLoading: boolean
  selectedFileRetry: () => void
  selectedPath: string | null
}) {
  const workspaceTabs = useWorkspaceTabs({
    activeId: selectedPath,
  })
  const fileNavigatorRef = useRef<HTMLDivElement>(null)
  const openPath = workspaceTabs.state.openIds.includes(selectedPath ?? '')
    ? selectedPath
    : null

  // Closing the last tab keeps the route pointing at the file it was showing:
  // an empty workspace is this session's state, not something worth sharing.
  function selectFile(path: string, pinned: boolean) {
    workspaceTabs.open(displayRouteFilePath(path), pinned)
    onSelectFilePath(path)
  }


  return (
    <section>
      <div className="grid min-w-0 lg:min-h-[calc(100dvh-var(--app-chrome))] lg:grid-cols-[minmax(300px,0.36fr)_minmax(0,0.64fr)]">
        <div
          aria-label="Repository file navigator"
          className="min-w-0 border-b border-border px-3 py-3 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring lg:border-b-0 lg:border-r lg:px-5"
          ref={fileNavigatorRef}
          tabIndex={-1}
        >
          {content ? (
            <div className="scope-content-enter">
              <RepositoryFileNavigator
                files={content.files}
                onSelectFile={selectFile}
                selectedPath={selectedRouteFilePath(
                  content.files,
                  openPath ?? undefined,
                )}
              />
            </div>
          ) : contentError ? (
            <FileNavigatorError error={contentError} retry={contentRetry} />
          ) : (
            <PendingSurface
              className="min-h-[220px]"
              delay
              label="Loading repository files"
            >
              <FileNavigatorSkeleton />
            </PendingSurface>
          )}
        </div>
        <SourcePane
          availablePaths={content
            ? content.files.map((file) => displayRouteFilePath(file.path))
            : workspaceTabs.state.openIds}
          error={selectedFileError}
          file={selectedFile}
          loading={selectedFileLoading}
          onActivateTab={onSelectFilePath}
          onEmptyTabFocus={() => fileNavigatorRef.current?.focus()}
          onPinTab={(path) => workspaceTabs.open(path, true)}
          params={params}
          retry={selectedFileRetry}
          scrollKey={selectedFileIdentity}
          selectedPath={openPath}
          workspaceTabs={workspaceTabs}
        />
      </div>
    </section>
  )
}

function RepositoryFileNavigator({
  files,
  onSelectFile,
  selectedPath,
}: {
  files: RepoFile[]
  onSelectFile: (path: string, pinned: boolean) => void
  selectedPath: string | null
}) {
  if (files.length === 0) {
    return (
      <EmptyState
        description="Run scope push from the CLI to add files to this repository."
        icon={<FileQuestion />}
        title="No files yet"
      />
    )
  }

  return (
    <FileSystemTree
      compactVisibility
      files={files}
      getFileMeta={fileStatus}
      metaColumnLabel="Status"
      onActivateFile={(file) => onSelectFile(file.path, true)}
      onSelectFile={(file) => onSelectFile(file.path, false)}
      selectedFilePath={selectedPath}
    />
  )
}

function FileNavigatorError({
  error,
  retry,
}: {
  error: string
  retry: () => void
}) {
  return (
    <PanelState role="alert" tone="error">
      <TriangleAlert className="size-5" />
      <span>{error}</span>
      <Button onClick={retry} size="sm" type="button" variant="secondary">
        Retry
      </Button>
    </PanelState>
  )
}

function SourcePane({
  availablePaths,
  error,
  file,
  loading,
  onActivateTab,
  onEmptyTabFocus,
  onPinTab,
  params,
  retry,
  scrollKey,
  selectedPath,
  workspaceTabs,
}: {
  availablePaths: string[]
  error: string | null
  file: RepoFileContent | null
  loading: boolean
  onActivateTab: (path: string) => void
  onEmptyTabFocus: () => void
  onPinTab: (path: string) => void
  params: RepoParams
  retry: () => void
  scrollKey: string | null
  selectedPath: string | null
  workspaceTabs: ReturnType<typeof useWorkspaceTabs>
}) {
  const activeTabDomIds = selectedPath
    ? workspaceTabDomIds(CODE_TAB_SET_ID, selectedPath)
    : null
  const contentRef = useRef<HTMLDivElement>(null)
  const meta = useMemo(
    () => file && selectedPath && !loading && !error
      ? <FileMeta file={file} />
      : undefined,
    [error, file, loading, selectedPath],
  )

  useLayoutEffect(() => {
    if (contentRef.current) {
      contentRef.current.scrollTop = readRepositorySourceScroll(scrollKey)
    }
  }, [scrollKey])

  return (
    <div className="min-w-0">
      <RepositoryTabStrip
        availablePaths={availablePaths}
        meta={meta}
        onActivateTab={onActivateTab}
        onEmptyTabFocus={onEmptyTabFocus}
        onPinTab={onPinTab}
        selectedPath={selectedPath}
        workspaceTabs={workspaceTabs}
      />
      <div
        aria-label={activeTabDomIds ? undefined : 'Repository file viewer'}
        aria-labelledby={activeTabDomIds?.tabId}
        className="max-h-[calc(100dvh-var(--app-chrome)-84px)] overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        id={workspaceTabPanelId(CODE_TAB_SET_ID)}
        onScroll={(event) =>
          writeRepositorySourceScroll(scrollKey, event.currentTarget.scrollTop)
        }
        ref={contentRef}
        role={selectedPath ? 'tabpanel' : undefined}
        tabIndex={selectedPath ? 0 : undefined}
      >
        <SourceContent
          error={error}
          file={file}
          loading={loading}
          params={params}
          retry={retry}
          selectedPath={selectedPath}
        />
      </div>
    </div>
  )
}

function RepositoryTabStrip({
  availablePaths,
  meta,
  onActivateTab,
  onEmptyTabFocus,
  onPinTab,
  selectedPath,
  workspaceTabs,
}: {
  availablePaths: string[]
  meta: ReactNode
  onActivateTab: (path: string) => void
  onEmptyTabFocus: () => void
  onPinTab: (path: string) => void
  selectedPath: string | null
  workspaceTabs: ReturnType<typeof useWorkspaceTabs>
}) {
  const items = availablePaths.map(workspaceTabItem)
  const itemById = new Map(items.map((item) => [item.id, item]))
  const availableIds = new Set(itemById.keys())
  const openState = pruneWorkspaceTabs(workspaceTabs.state, availableIds)
  const tabs = openState.openIds.flatMap((id) => {
    const item = itemById.get(id)
    return item ? [item] : []
  })

  function closeTab(id: string) {
    const result = workspaceTabs.close(id, availableIds)
    if (id === selectedPath && result.activeId) onActivateTab(result.activeId)
    return result.focusId
  }

  return (
    <WorkspaceTabStrip
      activeId={selectedPath}
      ariaLabel="Open repository files"
      meta={meta}
      onActivate={onActivateTab}
      onClose={closeTab}
      onEmptyFocus={onEmptyTabFocus}
      onPin={onPinTab}
      previewId={openState.previewId}
      tabSetId={CODE_TAB_SET_ID}
      tabs={tabs}
    />
  )
}

function SourceContent({
  error,
  file,
  loading,
  params,
  retry,
  selectedPath,
}: {
  error: string | null
  file: RepoFileContent | null
  loading: boolean
  params: RepoParams
  retry: () => void
  selectedPath: string | null
}) {
  if (!selectedPath) {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>Select a file to inspect its projected contents.</span>
      </PanelState>
    )
  }

  if (loading) {
    return (
      <PendingSurface
        className="min-h-[220px]"
        delay
        label={`Loading ${displayPath(selectedPath)}`}
      >
        <SourceCodeSkeleton />
      </PendingSurface>
    )
  }

  if (error) {
    return (
      <PanelState role="alert" tone="error">
        <TriangleAlert className="size-5" />
        <span>{error}</span>
        <Button onClick={retry} size="sm" type="button" variant="secondary">
          Retry
        </Button>
      </PanelState>
    )
  }

  if (!file) {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>This file is no longer available in the current scoped view.</span>
      </PanelState>
    )
  }

  return (
    <div className="scope-content-enter min-h-full" key={file.oid}>
      <SourceFileContent file={file} params={params} />
    </div>
  )
}

function FileMeta({ file }: { file: RepoFileContent }) {
  return (
    <>
      <span>
        {formatBytes(file.size_bytes)} · {file.oid.slice(0, 12)}
      </span>
      <VisibilityBadge compact visibility={file.visibility} />
    </>
  )
}

function SourceFileContent({
  file,
  params,
}: {
  file: RepoFileContent
  params: RepoParams
}) {
  if (file.content.kind !== 'text') {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>
          Binary file not rendered ·{' '}
          {formatBytes(file.content.size_bytes)} ·{' '}
          {file.content.oid.slice(0, 12)}
        </span>
      </PanelState>
    )
  }

  if (isRepositoryMarkdownPath(file.path)) {
    return (
      <RepositoryMarkdownRenderer
        repository={{ ...params, markdownPath: file.path }}
        source={file.content.text}
      />
    )
  }

  if (isRepositoryHtmlPath(file.path)) {
    return (
      <RepositoryHtmlRenderer
        identity={`${file.path}\0${file.oid}`}
        key={`${file.path}:${file.oid}`}
        path={file.path}
        source={file.content.text}
      />
    )
  }

  return (
    <pre className="min-h-full bg-background p-5 font-mono text-xs leading-5 whitespace-pre text-foreground sm:p-7">
      <code>{file.content.text}</code>
    </pre>
  )
}

function fileStatus(file: RepoFile) {
  return <span className="text-muted-foreground">{file.tracked ? 'Tracked' : 'Missing'}</span>
}

function displayPath(path: string) {
  return path.replace(/^\/+/, '') || '/'
}

function fileName(path: string) {
  return displayPath(path).split('/').at(-1) ?? displayPath(path)
}

function workspaceTabItem(path: string): WorkspaceTabItem {
  return {
    id: path,
    label: fileName(path),
    title: displayPath(path),
  }
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
