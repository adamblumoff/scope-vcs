import type { CreateRepoInput, Visibility } from '@/api/types'
import { Button } from '@/components/ui/button'
import { LoaderCircle, Plus } from 'lucide-react'
import type { FormEvent } from 'react'
import { useState } from 'react'

export function CreateRepoForm({
  onCreate,
}: {
  onCreate: (input: CreateRepoInput) => Promise<void>
}) {
  const [busy, setBusy] = useState(false)
  const [name, setName] = useState('')
  const [visibility, setVisibility] = useState<Visibility>('Private')

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!name.trim()) {
      return
    }

    setBusy(true)
    try {
      await onCreate({ name, visibility })
      setName('')
      setVisibility('Private')
    } finally {
      setBusy(false)
    }
  }

  return (
    <form
      className="grid w-full gap-2 sm:w-auto sm:grid-cols-[180px_120px_auto]"
      onSubmit={(event) => void submit(event)}
    >
      <input
        aria-label="Repository name"
        className="h-9 min-w-0 rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        disabled={busy}
        onChange={(event) => setName(event.target.value)}
        placeholder="new-repo"
        value={name}
      />
      <select
        aria-label="Default new file visibility"
        className="h-9 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        disabled={busy}
        onChange={(event) => setVisibility(event.target.value as Visibility)}
        value={visibility}
      >
        <option value="Private">Private</option>
        <option value="Public">Public</option>
      </select>
      <Button disabled={busy || !name.trim()} size="sm" type="submit">
        {busy ? (
          <LoaderCircle className="size-3.5 animate-spin" />
        ) : (
          <Plus className="size-3.5" />
        )}
        <span>Create</span>
      </Button>
    </form>
  )
}
