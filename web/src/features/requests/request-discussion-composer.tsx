import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { MessageSquarePlus, Reply, RotateCcw, Send, X } from 'lucide-react'
import { type FormEvent, type ReactNode, useId, useState } from 'react'

export function RequestDiscussionComposer({
  onSubmit,
}: {
  onSubmit: (body: string) => Promise<boolean>
}) {
  return (
    <Composer
      label="Start a new discussion"
      onSubmit={onSubmit}
      placeholder="Start a focused discussion about this request…"
      submitIcon={<MessageSquarePlus className="size-3.5" />}
      submitLabel="Start discussion"
    />
  )
}

export function RequestReplyComposer({
  onCancelQuote,
  onSubmit,
  quote,
  reopen,
}: {
  onCancelQuote: () => void
  onSubmit: (body: string) => Promise<boolean>
  quote: { author: string; body: string } | null
  reopen: boolean
}) {
  return (
    <Composer
      label={reopen ? 'Reopen and reply' : 'Reply'}
      onSubmit={onSubmit}
      placeholder={
        reopen
          ? 'Explain why this discussion needs to continue…'
          : 'Add a reply…'
      }
      quote={quote}
      onCancelQuote={onCancelQuote}
      submitIcon={
        reopen ? (
          <RotateCcw className="size-3.5" />
        ) : (
          <Reply className="size-3.5" />
        )
      }
      submitLabel={reopen ? 'Reopen and reply' : 'Reply'}
    />
  )
}

function Composer({
  label,
  onCancelQuote,
  onSubmit,
  placeholder,
  quote,
  submitIcon,
  submitLabel,
}: {
  label: string
  onCancelQuote?: () => void
  onSubmit: (body: string) => Promise<boolean>
  placeholder: string
  quote?: { author: string; body: string } | null
  submitIcon: ReactNode
  submitLabel: string
}) {
  const [body, setBody] = useState('')
  const [pending, setPending] = useState(false)
  const composerId = useId()

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const normalized = body.trim()
    if (!normalized || pending) return
    setPending(true)
    try {
      if (await onSubmit(normalized)) {
        setBody('')
        onCancelQuote?.()
      }
    } finally {
      setPending(false)
    }
  }

  return (
    <form className="border-t border-border pt-3" onSubmit={submit}>
      <label className="sr-only" htmlFor={composerId}>
        {label}
      </label>
      {quote ? (
        <div className="mb-2 flex min-w-0 items-start gap-2 border-l-2 border-border-strong pl-3 text-xs leading-5 text-muted-foreground">
          <div className="min-w-0 flex-1">
            <span className="font-medium text-foreground">{quote.author}</span>
            <span className="ml-1 line-clamp-1">{quote.body}</span>
          </div>
          <button
            aria-label="Cancel quoted reply"
            className="shrink-0 p-1 hover:text-foreground"
            onClick={onCancelQuote}
            type="button"
          >
            <X className="size-3.5" />
          </button>
        </div>
      ) : null}
      <textarea
        className={cn(
          'min-h-24 w-full resize-y rounded-md border border-input bg-background',
          'px-3 py-2 text-sm leading-6 outline-none placeholder:text-muted-foreground',
          'focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50',
        )}
        id={composerId}
        onChange={(event) => setBody(event.target.value)}
        placeholder={placeholder}
        value={body}
      />
      <div className="mt-2 flex items-center justify-between gap-3">
        <p className="text-xs text-muted-foreground">Markdown supported</p>
        <Button disabled={!body.trim() || pending} size="sm" type="submit">
          {submitIcon}
          {pending ? 'Posting…' : submitLabel}
        </Button>
      </div>
    </form>
  )
}
