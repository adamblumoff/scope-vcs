import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useMemo, useState } from 'react'
import { repositoryHtmlDocument } from './repository-html'
import { PersistentRepositoryHtmlPreview } from './repository-html-preview-store'

type RepositoryHtmlMode = 'preview' | 'source'

export function RepositoryHtmlRenderer({
  identity,
  path,
  source,
}: {
  identity: string
  path: string
  source: string
}) {
  const [mode, setMode] = useState<RepositoryHtmlMode>('preview')
  const document = useMemo(() => repositoryHtmlDocument(source), [source])
  const displayPath = path.replace(/^\/+/, '')

  function selectMode(value: string) {
    if (value === 'preview' || value === 'source') setMode(value)
  }

  return (
    <div className="min-w-0">
      <div className="flex min-h-11 items-center justify-between gap-4 border-b border-border px-5 py-1.5 sm:px-8">
        <span className="truncate font-mono text-[11px] text-muted-foreground">
          Sandboxed document
        </span>
        <ToggleGroup
          aria-label={`${displayPath} display mode`}
          onValueChange={selectMode}
          size="sm"
          type="single"
          value={mode}
        >
          <ToggleGroupItem className="h-7 px-2 text-xs" value="preview">
            Preview
          </ToggleGroupItem>
          <ToggleGroupItem className="h-7 px-2 text-xs" value="source">
            Source
          </ToggleGroupItem>
        </ToggleGroup>
      </div>
      {mode === 'preview' ? (
        <PersistentRepositoryHtmlPreview
          className="h-[calc(100dvh-var(--app-chrome)-132px)] min-h-[32rem] max-h-[70rem] w-full border-0 bg-white"
          identity={identity}
          srcDoc={document}
          title={`${displayPath} preview`}
        />
      ) : (
        <pre className="min-h-[32rem] overflow-x-auto bg-background p-5 font-mono text-xs leading-5 whitespace-pre text-foreground sm:p-7">
          <code>{source}</code>
        </pre>
      )}
    </div>
  )
}
