import type { RepoRunStep } from '@/api/types'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Check, Copy, TerminalSquare, WrapText } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { StepLogState } from './repository-run-detail-controller'

const FOLLOW_THRESHOLD_PX = 32
const COPY_CONFIRMATION_MS = 1_500

/** A single step's output: follows new lines while the step runs, wraps or
 * scrolls horizontally on request, and copies the buffered text to the
 * clipboard. */
export function RunLogView({
  id,
  logState,
  onRetry,
  step,
}: {
  id: string
  logState: StepLogState
  onRetry: () => void
  step: RepoRunStep
}) {
  const [wrap, setWrap] = useState(true)
  const [following, setFollowing] = useState(true)
  const [copied, setCopied] = useState(false)
  const scrollRef = useRef<HTMLPreElement>(null)
  const isRunning = step.state === 'running'

  useEffect(() => {
    setFollowing(true)
  }, [id])

  useEffect(() => {
    if (!isRunning || !following) return
    const node = scrollRef.current
    if (!node) return
    node.scrollTop = node.scrollHeight
  }, [following, isRunning, logState.logs])

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), COPY_CONFIRMATION_MS)
    return () => clearTimeout(timer)
  }, [copied])

  function handleScroll() {
    const node = scrollRef.current
    if (!node) return
    const distanceFromBottom = node.scrollHeight - node.scrollTop - node.clientHeight
    setFollowing(distanceFromBottom <= FOLLOW_THRESHOLD_PX)
  }

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(
        logState.logs.map((log) => log.text).join(''),
      )
      setCopied(true)
    } catch {
      // The browser denied clipboard access; there is nothing to recover.
    }
  }

  return (
    <section
      aria-label={`${step.name} output`}
      className="border-t border-border bg-background text-foreground"
      id={id}
    >
      <div className="flex min-h-10 flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-2 text-xs text-muted-foreground">
        <span className="flex items-center gap-2">
          <TerminalSquare className="size-3.5" />
          {step.name}
          {step.exit_code !== null ? ` · exit ${step.exit_code}` : ''}
        </span>
        <span className="flex items-center gap-3">
          <span>{logStatusLabel(logState, isRunning, following)}</span>
          {logState.logsTruncated ? <span>Some output omitted</span> : null}
          <Button
            aria-pressed={wrap}
            onClick={() => setWrap((value) => !value)}
            size="icon-xs"
            title={wrap ? 'Disable line wrap' : 'Wrap long lines'}
            variant="ghost"
          >
            <WrapText />
          </Button>
          <Button
            disabled={logState.logs.length === 0}
            onClick={() => void handleCopy()}
            size="icon-xs"
            title="Copy output"
            variant="ghost"
          >
            {copied ? <Check /> : <Copy />}
          </Button>
        </span>
      </div>
      {logState.error ? (
        <div
          className="flex flex-wrap items-center gap-3 border-b border-border px-4 py-3 text-sm text-danger-strong"
          role="alert"
        >
          <span>{logState.error}</span>
          <Button onClick={onRetry} size="sm" variant="secondary">
            Retry logs
          </Button>
        </div>
      ) : null}
      <pre
        className={cn(
          'max-h-[34rem] overflow-auto break-words px-4 py-4 font-mono text-xs leading-5',
          wrap ? 'whitespace-pre-wrap' : 'whitespace-pre',
        )}
        onScroll={handleScroll}
        ref={scrollRef}
      >
        {logState.logs.length === 0
          ? <span className="text-muted-foreground">No output yet.</span>
          : logState.logs.map((log) => log.text).join('')}
      </pre>
    </section>
  )
}

function logStatusLabel(
  logState: StepLogState,
  isRunning: boolean,
  following: boolean,
) {
  if (logState.loading) return 'Fetching new output…'
  if (isRunning) {
    return following ? 'Following live output' : 'Paused, scroll down to follow'
  }
  return 'Output finished'
}
