import type { RequestSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { LoaderCircle, UserMinus, UserPlus } from 'lucide-react'
import type { FormEvent } from 'react'
import { useId, useState } from 'react'
import type { RequestActionController } from './use-request-actions'

export function RequestInvitees({
  actions,
  request,
}: {
  actions: RequestActionController
  request: RequestSummary
}) {
  const inputId = useId()
  const [handle, setHandle] = useState('')
  const [removingHandle, setRemovingHandle] = useState<string | null>(null)
  const normalizedHandle = handle.trim().replace(/^@/, '')
  const atCapacity = request.invitees.length >= 30
  const adding = actions.pending === 'add_invitee'

  async function addInvitee(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!normalizedHandle || atCapacity || adding) return
    if (await actions.run({ action: 'add_invitee', handle: normalizedHandle })) {
      setHandle('')
    }
  }

  async function removeInvitee(handle: string) {
    setRemovingHandle(handle)
    await actions.run({ action: 'remove_invitee', handle })
    setRemovingHandle(null)
  }

  return (
    <section className="border-b border-border px-5 py-5">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground">
          Invitees
        </h2>
        <span className="font-mono text-xs text-muted-foreground">
          {request.invitees.length} / 30
        </span>
      </div>
      <p className="mt-2 text-xs leading-5 text-muted-foreground">
        Exact-handle collaborators can push this public request branch. They cannot edit its identity or assess it.
      </p>

      {request.invitees.length > 0 ? (
        <div className="mt-3 divide-y divide-border border-y border-border">
          {request.invitees.map((invitee) => (
            <div className="flex min-w-0 items-center gap-2 py-2" key={invitee.user.id}>
              <span className="min-w-0 flex-1 truncate text-sm">@{invitee.user.handle}</span>
              <span className="text-xs text-muted-foreground">push</span>
              {request.permissions.can_manage_invitees ? (
                <Button
                  aria-label={`Remove @${invitee.user.handle}`}
                  disabled={actions.pending !== null}
                  onClick={() => void removeInvitee(invitee.user.handle)}
                  size="icon-sm"
                  title={`Remove @${invitee.user.handle}`}
                  type="button"
                  variant="ghost"
                >
                  {removingHandle === invitee.user.handle ? (
                    <LoaderCircle className="animate-spin" />
                  ) : (
                    <UserMinus />
                  )}
                </Button>
              ) : null}
            </div>
          ))}
        </div>
      ) : (
        <p className="mt-3 text-sm text-muted-foreground">No invitees.</p>
      )}

      {request.permissions.can_manage_invitees ? (
        <form className="mt-3 grid gap-2" onSubmit={(event) => void addInvitee(event)}>
          <label className="text-xs font-medium" htmlFor={inputId}>
            Add exact Scope handle
          </label>
          <div className="flex gap-2">
            <Input
              autoComplete="off"
              disabled={atCapacity || actions.pending !== null}
              id={inputId}
              onChange={(event) => setHandle(event.target.value)}
              placeholder="@handle"
              value={handle}
            />
            <Button
              aria-label="Add request invitee"
              disabled={!normalizedHandle || atCapacity || actions.pending !== null}
              size="icon-sm"
              title="Add invitee"
              type="submit"
              variant="secondary"
            >
              {adding ? <LoaderCircle className="animate-spin" /> : <UserPlus />}
            </Button>
          </div>
          {atCapacity ? (
            <p className="text-xs text-muted-foreground">The 30-invitee limit has been reached.</p>
          ) : null}
        </form>
      ) : null}

      {request.permissions.can_leave_request ? (
        <Button
          className="mt-3"
          disabled={actions.pending !== null}
          onClick={() => void actions.run({ action: 'leave' })}
          size="sm"
          type="button"
          variant="secondary"
        >
          {actions.pending === 'leave' ? <LoaderCircle className="animate-spin" /> : null}
          Leave request
        </Button>
      ) : null}
    </section>
  )
}
