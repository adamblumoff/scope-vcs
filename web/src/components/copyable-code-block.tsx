import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { Check, Copy } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

type CopyableCodeBlockProps = {
  className?: string
  copyLabel?: string
  onCopy?: () => void
  value: string
}

export function CopyableCodeBlock({
  className,
  copyLabel = 'Copy',
  onCopy,
  value,
}: CopyableCodeBlockProps) {
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) {
      return
    }

    const timeout = window.setTimeout(() => setCopied(false), 1200)
    return () => window.clearTimeout(timeout)
  }, [copied])

  async function copyToClipboard() {
    let clipboardError: unknown

    if (navigator.clipboard?.writeText) {
      try {
        await navigator.clipboard.writeText(value)
        markCopied()
        return
      } catch (error) {
        clipboardError = error
      }
    }

    if (copyWithFallback(value)) {
      markCopied()
      return
    }

    console.error(
      'copy failed',
      clipboardError ?? new Error('clipboard unavailable'),
    )
    toast.error('Copy failed')
  }

  function markCopied() {
    setCopied(true)
    toast.success('Copied')
    onCopy?.()
  }

  return (
    <div
      className={cn(
        'relative rounded-lg border border-border border-l-2 border-l-[var(--platinum)] bg-[#090b0e] text-[#eceae5] shadow-[var(--shadow-card)]',
        className,
      )}
    >
      <pre className="overflow-x-auto whitespace-pre-wrap break-words px-3 py-2 pr-12 font-mono text-xs leading-5 [overflow-wrap:anywhere]">
        <code>{value}</code>
      </pre>
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label={copied ? 'Copied' : copyLabel}
              className="absolute inset-y-0 right-2 my-auto border-white/15 bg-white/5 text-[#aeb4bf] hover:bg-white/10 hover:text-white"
              onClick={() => void copyToClipboard()}
              size="icon-sm"
              type="button"
              variant="secondary"
            >
              {copied ? (
                <Check className="size-3.5" />
              ) : (
                <Copy className="size-3.5" />
              )}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{copied ? 'Copied' : copyLabel}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </div>
  )
}

function copyWithFallback(value: string): boolean {
  if (typeof document === 'undefined') {
    return false
  }

  const textarea = document.createElement('textarea')
  textarea.value = value
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.top = '-9999px'
  textarea.style.opacity = '0'
  document.body.appendChild(textarea)
  textarea.select()

  let copied = false
  try {
    copied = document.execCommand('copy')
  } catch {
    copied = false
  } finally {
    document.body.removeChild(textarea)
  }

  return copied
}
