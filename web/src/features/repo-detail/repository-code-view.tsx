import type { RepoFile, RepoFileContent, RepoParams } from '@/api/types'
import { EmptyState, PanelState } from '@/components/empty-state'
import { FileSystemTree } from '@/components/file-system-tree'
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
  type WorkspaceTabItem,
} from '@/components/workspace-tab-model'
import { FileQuestion, LoaderCircle, TriangleAlert } from 'lucide-react'
import { useLayoutEffect, useMemo, useRef } from 'react'
import {
  readRepositorySourceScroll,
  writeRepositorySourceScroll,
} from './repository-source-scroll-cache'

const CODE_TAB_SET_ID = 'repository-code-files'

export function RepositoryCodeView({
  files,
  onSelectFilePath,
  params,
  selectedFile,
  selectedFileError,
  selectedFileIdentity,
  selectedFileLoading,
  selectedFileRetry,
  selectedPath,
}: {
  files: RepoFile[]
  onSelectFilePath: (path: string) => void
  params: RepoParams
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedFileIdentity: string | null
  selectedFileLoading: boolean
  selectedFileRetry: () => void
  selectedPath: string | null
}) {
  const tabItems = useMemo(
    () =>
      files.map((file) => ({
        id: file.path,
        label: fileName(file.path),
        title: displayPath(file.path),
      })),
    [files],
  )
  const workspaceTabs = useWorkspaceTabs({
    activeId: selectedPath,
    items: tabItems,
  })
  const fileNavigatorRef = useRef<HTMLDivElement>(null)
  const openPath = workspaceTabs.tabs.some((tab) => tab.id === selectedPath)
    ? selectedPath
    : null

  // Closing the last tab keeps the route pointing at the file it was showing:
  // an empty workspace is this session's state, not something worth sharing.
  function closeTab(id: string) {
    const result = workspaceTabs.close(id)
    if (id === selectedPath && result.activeId) onSelectFilePath(result.activeId)
    return result.focusId
  }

  function selectFile(path: string, pinned: boolean) {
    workspaceTabs.open(path, pinned)
    onSelectFilePath(path)
  }

  return (
    <section>
      {files.length === 0 ? (
        <EmptyState
          description="Run scope push from the CLI to add files to this repository."
          icon={<FileQuestion />}
          title="No files yet"
        />
      ) : (
        <div className="grid min-w-0 lg:min-h-[calc(100dvh-var(--app-chrome))] lg:grid-cols-[minmax(300px,0.36fr)_minmax(0,0.64fr)]">
          <div
            aria-label="Repository file navigator"
            className="min-w-0 border-b border-border px-3 py-3 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring lg:border-b-0 lg:border-r lg:px-5"
            ref={fileNavigatorRef}
            tabIndex={-1}
          >
            <FileSystemTree
              compactVisibility
              files={files}
              getFileMeta={fileStatus}
              metaColumnLabel="Status"
              onActivateFile={(file) => selectFile(file.path, true)}
              onSelectFile={(file) => selectFile(file.path, false)}
              selectedFilePath={openPath}
            />
          </div>
          <SourcePane
            error={selectedFileError}
            file={selectedFile}
            loading={selectedFileLoading}
            onActivateTab={onSelectFilePath}
            onCloseTab={closeTab}
            onEmptyTabFocus={() => fileNavigatorRef.current?.focus()}
            onPinTab={(path) => workspaceTabs.open(path, true)}
            params={params}
            previewId={workspaceTabs.previewId}
            retry={selectedFileRetry}
            scrollKey={selectedFileIdentity}
            selectedPath={openPath}
            tabs={workspaceTabs.tabs}
          />
        </div>
      )}
    </section>
  )
}

function SourcePane({
  file,
  error,
  loading,
  onActivateTab,
  onCloseTab,
  onEmptyTabFocus,
  onPinTab,
  params,
  previewId,
  retry,
  scrollKey,
  selectedPath,
  tabs,
}: {
  file: RepoFileContent | null
  error: string | null
  loading: boolean
  onActivateTab: (path: string) => void
  onCloseTab: (path: string) => string | null
  onEmptyTabFocus: () => void
  onPinTab: (path: string) => void
  params: RepoParams
  previewId: string | null
  retry: () => void
  scrollKey: string | null
  selectedPath: string | null
  tabs: WorkspaceTabItem[]
}) {
  const activeTabDomIds = selectedPath
    ? workspaceTabDomIds(CODE_TAB_SET_ID, selectedPath)
    : null
  const contentRef = useRef<HTMLDivElement>(null)
  const meta = useMemo(
    () =>
      file && selectedPath && !loading && !error ? (
        <FileMeta file={file} />
      ) : undefined,
    [error, file, loading, selectedPath],
  )

  useLayoutEffect(() => {
    if (contentRef.current) {
      contentRef.current.scrollTop = readRepositorySourceScroll(scrollKey)
    }
  }, [scrollKey])

  return (
    <div className="min-w-0">
      <WorkspaceTabStrip
        activeId={selectedPath}
        ariaLabel="Open repository files"
        meta={meta}
        onActivate={onActivateTab}
        onClose={onCloseTab}
        onEmptyFocus={onEmptyTabFocus}
        onPin={onPinTab}
        previewId={previewId}
        tabSetId={CODE_TAB_SET_ID}
        tabs={tabs}
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
        role={tabs.length > 0 ? 'tabpanel' : undefined}
        tabIndex={tabs.length > 0 ? 0 : undefined}
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

function SourceContent({
  file,
  error,
  loading,
  params,
  retry,
  selectedPath,
}: {
  file: RepoFileContent | null
  error: string | null
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
      <PanelState busy>
        <LoaderCircle className="size-5 animate-spin" />
        <span>Loading {displayPath(selectedPath)}</span>
      </PanelState>
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

  return <SourceFileContent file={file} params={params} />
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
    <pre className="min-h-full bg-[var(--terminal-surface)] p-5 font-mono text-xs leading-5 whitespace-pre text-[var(--terminal-foreground)] sm:p-7">
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

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
