import type { RepoFile, RepoFileContent, RepoParams } from '@/api/types'
import { FileSystemTree } from '@/components/file-system-tree'
import { ReadmeRenderer } from '@/components/readme-renderer'
import { VisibilityBadge } from '@/components/visibility-badge'
import { FileCode2, FileQuestion, TriangleAlert } from 'lucide-react'

export function RepositoryCodeView({
  files,
  onSelectFile,
  params,
  selectedFile,
  selectedFileError,
  selectedPath,
}: {
  files: RepoFile[]
  onSelectFile: (file: RepoFile) => void
  params: RepoParams
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedPath: string | null
}) {
  return (
    <section>
      {files.length === 0 ? (
        <EmptySource
          description="Run scope push from the CLI to add files to this repository."
          title="No files yet"
        />
      ) : (
        <div className="grid min-w-0 lg:min-h-[calc(100dvh-225px)] lg:grid-cols-[minmax(300px,0.36fr)_minmax(0,0.64fr)]">
          <div className="min-w-0 border-b border-border px-3 py-3 lg:border-b-0 lg:border-r lg:px-5">
            <FileSystemTree
              compactVisibility
              files={files}
              getFileMeta={fileStatus}
              metaColumnLabel="Status"
              onSelectFile={onSelectFile}
              selectedFilePath={selectedPath}
            />
          </div>
          <SourcePane
            error={selectedFileError}
            file={selectedFile}
            params={params}
            selectedPath={selectedPath}
          />
        </div>
      )}
    </section>
  )
}

function SourcePane({
  file,
  error,
  params,
  selectedPath,
}: {
  file: RepoFileContent | null
  error: string | null
  params: RepoParams
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

  if (error) {
    return (
      <div className="flex min-h-52 items-center justify-center px-5 py-10 text-center" role="alert">
        <div className="max-w-sm">
          <TriangleAlert className="mx-auto size-5 text-destructive" />
          <h3 className="mt-3 text-sm font-semibold">Source unavailable</h3>
          <p className="mt-1 text-sm leading-5 text-muted-foreground">{error}</p>
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
      <div className="flex min-h-[74px] min-w-0 flex-wrap items-center gap-3 border-b border-border px-5 py-3 sm:px-8">
        <FileCode2 className="size-[18px] shrink-0 text-[var(--platinum)]" strokeWidth={1.7} />
        <div className="min-w-0 flex-1">
          <h3 className="min-w-0 break-all font-mono text-sm font-semibold">
            {displayPath(file.path)}
          </h3>
          <div className="mt-1 text-xs text-muted-foreground">
            {file.content.kind === 'text' ? formatBytes(new TextEncoder().encode(file.content.text).length) : formatBytes(file.content.size_bytes)}
          </div>
        </div>
        <VisibilityBadge compact visibility={file.visibility} />
      </div>
      {file.content.kind === 'text' ? (
        isReadme(file.path) ? (
          <ReadmeRenderer
            repository={{ ...params, readmePath: file.path }}
            source={file.content.text}
          />
        ) : (
          <pre className="max-h-[calc(100dvh-300px)] overflow-auto bg-[#090b0e] p-5 font-mono text-xs leading-5 whitespace-pre text-[#eceae5] sm:p-7">
            <code>{file.content.text}</code>
          </pre>
        )
      ) : (
        <EmptySource
          description={`${formatBytes(file.content.size_bytes)} · ${file.content.oid.slice(0, 12)}`}
          title="Binary file not rendered"
        />
      )}
    </div>
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

function fileStatus(file: RepoFile) {
  return <span className="text-muted-foreground">{file.tracked ? 'Tracked' : 'Missing'}</span>
}

function displayPath(path: string) {
  return path.replace(/^\/+/, '') || '/'
}

function isReadme(path: string) {
  return /(^|\/)readme(?:\.[^/]+)?$/i.test(path)
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
