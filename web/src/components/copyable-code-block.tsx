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
  value: string
}

export function CopyableCodeBlock({
  className,
  copyLabel = 'Copy',
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
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(value)
      } else if (!copyWithFallback(value)) {
        throw new Error('clipboard unavailable')
      }
      setCopied(true)
      toast.success('Copied')
    } catch (error) {
      if (copyWithFallback(value)) {
        setCopied(true)
        toast.success('Copied')
        return
      }
      console.error('copy failed', error)
      toast.error('Copy failed')
    }
  }

  return (
    <div
      className={cn(
        'relative rounded-lg border border-border bg-muted',
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
              className="absolute right-2 top-2 bg-background/80 text-muted-foreground hover:bg-background hover:text-foreground"
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
