import type { RepoFile, RepoFileContent, RepoParams } from '@/api/types'
import { FileSystemTree } from '@/components/file-system-tree'
import { ReadmeRenderer } from '@/components/readme-renderer'
import { Button } from '@/components/ui/button'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import { VisibilityBadge } from '@/components/visibility-badge'
import { WorkspaceTabStrip } from '@/components/workspace-tab-strip'
import {
  workspaceTabDomIds,
  workspaceTabPanelId,
} from '@/components/workspace-tab-model'
import { FileQuestion, LoaderCircle, TriangleAlert } from 'lucide-react'
import { useLayoutEffect, useMemo, useRef } from 'react'

const MAX_SOURCE_SCROLL_POSITIONS = 64
const sourceScrollPositions = new Map<string, number>()

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
  onSelectFilePath: (path: string | null) => void
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
    storageKey: `code:${params.owner}/${params.repo}`,
  })
  const fileNavigatorRef = useRef<HTMLDivElement>(null)

  function closeTab(id: string) {
    const result = workspaceTabs.close(id)
    if (id === selectedPath) onSelectFilePath(result.activeId)
    return result.focusId
  }

  return (
    <section>
      {files.length === 0 ? (
        <EmptySource
          description="Run scope push from the CLI to add files to this repository."
          title="No files yet"
        />
      ) : (
        <div className="grid min-w-0 lg:min-h-[calc(100dvh-225px)] lg:grid-cols-[minmax(300px,0.36fr)_minmax(0,0.64fr)]">
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
              onSelectFile={(file) => {
                workspaceTabs.prepareOpen(file.path)
                onSelectFilePath(file.path)
              }}
              selectedFilePath={selectedPath}
            />
          </div>
          <SourcePane
            error={selectedFileError}
            file={selectedFile}
            loading={selectedFileLoading}
            onActivateTab={onSelectFilePath}
            onCloseTab={closeTab}
            onEmptyTabFocus={() => fileNavigatorRef.current?.focus()}
            params={params}
            retry={selectedFileRetry}
            scrollKey={selectedFileIdentity}
            selectedPath={selectedPath}
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
  params,
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
  params: RepoParams
  retry: () => void
  scrollKey: string | null
  selectedPath: string | null
  tabs: Array<{ id: string; label: string; title?: string }>
}) {
  const activeTabDomIds = selectedPath && tabs.some((tab) => tab.id === selectedPath)
    ? workspaceTabDomIds(CODE_TAB_SET_ID, selectedPath)
    : null
  const contentRef = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    if (contentRef.current) {
      contentRef.current.scrollTop = scrollKey
        ? (sourceScrollPositions.get(scrollKey) ?? 0)
        : 0
    }
  }, [scrollKey])

  return (
    <div className="min-w-0">
      <WorkspaceTabStrip
        activeId={selectedPath}
        ariaLabel="Open repository files"
        onActivate={onActivateTab}
        onClose={onCloseTab}
        onEmptyFocus={onEmptyTabFocus}
        tabSetId={CODE_TAB_SET_ID}
        tabs={tabs}
      />
      <div
        aria-label={activeTabDomIds ? undefined : 'Repository file viewer'}
        aria-labelledby={activeTabDomIds?.tabId}
        className="max-h-[calc(100dvh-300px)] overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        id={workspaceTabPanelId(CODE_TAB_SET_ID)}
        onScroll={(event) => {
          if (scrollKey) {
            saveSourceScrollPosition(scrollKey, event.currentTarget.scrollTop)
          }
        }}
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
      <EmptySource
        description="Choose a file from the tree to inspect its projected contents."
        title="Select a file"
      />
    )
  }

  if (loading) {
    return (
      <div
        aria-busy="true"
        className="flex min-h-52 items-center justify-center px-5 py-10 text-center"
      >
        <div className="max-w-sm">
          <LoaderCircle className="mx-auto size-5 animate-spin text-muted-foreground" />
          <h3 className="mt-3 text-sm font-semibold">Loading source</h3>
          <p className="mt-1 text-sm leading-5 text-muted-foreground">
            Fetching {displayPath(selectedPath)}.
          </p>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="flex min-h-52 items-center justify-center px-5 py-10 text-center" role="alert">
        <div className="max-w-sm">
          <TriangleAlert className="mx-auto size-5 text-destructive" />
          <h3 className="mt-3 text-sm font-semibold">Source unavailable</h3>
          <p className="mt-1 text-sm leading-5 text-muted-foreground">{error}</p>
          <Button className="mt-4" onClick={retry} size="sm" type="button" variant="secondary">
            Retry
          </Button>
        </div>
      </div>
    )
  }

  if (!file) {
    return (
      <EmptySource
        description="This file is no longer available in the current scoped view."
        title="File unavailable"
      />
    )
  }

  return (
    <div className="min-w-0">
      <div className="flex min-h-11 min-w-0 items-center gap-3 border-b border-border px-5 py-2 sm:px-8">
        <div className="min-w-0 flex-1 font-mono text-[11px] text-muted-foreground">
          {formatBytes(file.size_bytes)} · {file.oid.slice(0, 12)}
        </div>
        <VisibilityBadge compact visibility={file.visibility} />
      </div>
      <SourceFileContent file={file} params={params} />
    </div>
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
      <EmptySource
        description={`${formatBytes(file.content.size_bytes)} · ${file.content.oid.slice(0, 12)}`}
        title="Binary file not rendered"
      />
    )
  }

  if (isReadme(file.path)) {
    return (
      <ReadmeRenderer
        repository={{ ...params, readmePath: file.path }}
        source={file.content.text}
      />
    )
  }

  return (
    <pre className="min-h-full bg-[#090b0e] p-5 font-mono text-xs leading-5 whitespace-pre text-[#eceae5] sm:p-7">
      <code>{file.content.text}</code>
    </pre>
  )
}

function EmptySource({ description, title }: { description: string; title: string }) {
  return (
    <div className="flex min-h-52 items-center justify-center px-5 py-10 text-center">
      <div className="max-w-sm">
        <FileQuestion className="mx-auto size-5 text-muted-foreground" />
        <h3 className="mt-3 text-sm font-semibold">{title}</h3>
        <p className="mt-1 text-sm leading-5 text-muted-foreground">{description}</p>
      </div>
    </div>
  )
}

function saveSourceScrollPosition(key: string, scrollTop: number) {
  sourceScrollPositions.delete(key)
  sourceScrollPositions.set(key, scrollTop)

  while (sourceScrollPositions.size > MAX_SOURCE_SCROLL_POSITIONS) {
    const oldestKey = sourceScrollPositions.keys().next().value
    if (oldestKey === undefined) return
    sourceScrollPositions.delete(oldestKey)
  }
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

function isReadme(path: string) {
  return /(^|\/)readme(?:\.[^/]+)?$/i.test(path)
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
