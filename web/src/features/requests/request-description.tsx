import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Check, FileText, Pencil, X } from 'lucide-react'
import { type FormEvent, useState } from 'react'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'

export function RequestDescription({
  canEdit,
  description,
  onSave,
}: {
  canEdit: boolean
  description: string
  onSave: (description: string) => Promise<boolean>
}) {
  const [draft, setDraft] = useState('')
  const [editing, setEditing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setPending(true)
    setError(null)
    try {
      if (await onSave(draft.trim())) setEditing(false)
      else setError('The request description could not be saved.')
    } finally {
      setPending(false)
    }
  }

  return (
    <section className="border-b border-border px-5 py-5 lg:px-7">
      <div className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-2">
          <FileText className="size-4 text-muted-foreground" />
          <h2 className="text-sm font-semibold">Request description</h2>
        </div>
        {canEdit && !editing ? (
          <Button
            onClick={() => {
              setDraft(description)
              setError(null)
              setEditing(true)
            }}
            size="sm"
            type="button"
            variant="ghost"
          >
            <Pencil className="size-3.5" />
            Edit
          </Button>
        ) : null}
      </div>

      {editing ? (
        <form className="mt-3" onSubmit={submit}>
          <textarea
            aria-label="Request description"
            className={cn(
              'min-h-36 w-full resize-y rounded-md border border-input bg-background',
              'px-3 py-2 text-sm leading-6 outline-none placeholder:text-muted-foreground',
              'focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50',
            )}
            onChange={(event) => setDraft(event.target.value)}
            placeholder="Explain the intent, approach, and how this request was tested."
            value={draft}
          />
          <div className="mt-2 flex items-center gap-2">
            <Button disabled={pending} size="sm" type="submit">
              <Check className="size-3.5" />
              {pending ? 'Saving…' : 'Save description'}
            </Button>
            <Button
              disabled={pending}
              onClick={() => {
                setDraft(description)
                setEditing(false)
              }}
              size="sm"
              type="button"
              variant="secondary"
            >
              <X className="size-3.5" />
              Cancel
            </Button>
          </div>
          {error ? (
            <p className="mt-2 text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </form>
      ) : description ? (
        <RequestDiscussionMarkdown className="mt-3 max-w-4xl" source={description} />
      ) : (
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          No description yet.
        </p>
      )}
    </section>
  )
}
