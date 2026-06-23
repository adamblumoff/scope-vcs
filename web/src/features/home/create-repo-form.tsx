import type { CreateRepoInput, Visibility } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
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
      <Input
        aria-label="Repository name"
        disabled={busy}
        onChange={(event) => setName(event.target.value)}
        placeholder="new-repo"
        value={name}
      />
      <Select
        disabled={busy}
        onValueChange={(value) => setVisibility(value as Visibility)}
        value={visibility}
      >
        <SelectTrigger
          aria-label="Default new file visibility"
          className="w-full"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="Private">Private</SelectItem>
          <SelectItem value="Public">Public</SelectItem>
        </SelectContent>
      </Select>
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
