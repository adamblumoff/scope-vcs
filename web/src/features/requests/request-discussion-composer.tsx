import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { MessageSquarePlus, Reply, RotateCcw, X } from 'lucide-react'
import {
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  useId,
  useState,
} from 'react'

/**
 * Sits at the end of the list, where a new discussion lands. Collapsed it is a
 * single line; activating it opens the full composer.
 */
export function RequestDiscussionComposer({
  onSubmit,
}: {
  onSubmit: (body: string) => Promise<boolean>
}) {
  const [open, setOpen] = useState(false)

  if (!open) {
    return (
      <button
        className={cn(
          'flex w-full items-center gap-2 rounded-md border border-input bg-background',
          'px-3 py-2 text-left text-sm text-muted-foreground',
          'hover:border-ring hover:text-foreground',
          'focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none',
        )}
        onClick={() => setOpen(true)}
        type="button"
      >
        <MessageSquarePlus className="size-3.5 shrink-0" />
        Start a discussion about this request…
      </button>
    )
  }

  return (
    <Composer
      autoFocus
      label="Start a new discussion"
      onCancel={() => setOpen(false)}
      onSubmit={async (body) => {
        const posted = await onSubmit(body)
        if (posted) setOpen(false)
        return posted
      }}
      placeholder="Start a focused discussion about this request…"
      submitIcon={<MessageSquarePlus className="size-3.5" />}
      submitLabel="Start discussion"
    />
  )
}

export function RequestReplyComposer({
  onCancel,
  onCancelQuote,
  onSubmit,
  quote,
  reopen,
}: {
  onCancel: () => void
  onCancelQuote: () => void
  onSubmit: (body: string) => Promise<boolean>
  quote: { author: string; body: string } | null
  reopen: boolean
}) {
  return (
    <Composer
      autoFocus
      label={reopen ? 'Reopen and reply' : 'Reply'}
      onCancel={onCancel}
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
  autoFocus = false,
  label,
  onCancel,
  onCancelQuote,
  onSubmit,
  placeholder,
  quote,
  submitIcon,
  submitLabel,
}: {
  autoFocus?: boolean
  label: string
  onCancel: () => void
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

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.nativeEvent.isComposing) return
    if (event.key === 'Escape') {
      event.preventDefault()
      if (quote && onCancelQuote) onCancelQuote()
      else onCancel()
      return
    }
    if (pending) return
    if (event.key !== 'Enter' || event.shiftKey) return
    event.preventDefault()
    event.currentTarget.form?.requestSubmit()
  }

  return (
    <form onSubmit={submit}>
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
          'disabled:cursor-wait disabled:opacity-70',
        )}
        autoFocus={autoFocus}
        disabled={pending}
        id={composerId}
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
        value={body}
      />
      <div className="mt-2 flex items-center justify-between gap-3">
        <p className="text-xs text-muted-foreground">
          Markdown · Shift+Enter for a new line
        </p>
        <div className="flex items-center gap-2">
          <Button disabled={pending} onClick={onCancel} size="sm" type="button" variant="ghost">
            Cancel
          </Button>
          <Button disabled={!body.trim() || pending} size="sm" type="submit">
            {submitIcon}
            {pending ? 'Posting…' : submitLabel}
          </Button>
        </div>
      </div>
    </form>
  )
}
