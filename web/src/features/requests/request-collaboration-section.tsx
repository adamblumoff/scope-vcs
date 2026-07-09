import type { RequestSummary } from '@/api/types'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { GitBranch, UserPlus, X } from 'lucide-react'
import type { FormEvent, ReactNode } from 'react'

type RequestCollaborationActionKey = 'add-editor' | 'remove-editor'

export type RequestCollaborationError = {
  key: RequestCollaborationActionKey | string
  message: string
}

export function RequestCollaborationSection({
  actionError,
  editorUserId,
  onAddEditor,
  onEditorUserIdChange,
  onRemoveEditor,
  pendingAction,
  request,
}: {
  actionError: RequestCollaborationError | null
  editorUserId: string
  onAddEditor: (event: FormEvent<HTMLFormElement>) => void
  onEditorUserIdChange: (value: string) => void
  onRemoveEditor: (editorUserId: string) => void
  pendingAction: string | null
  request: RequestSummary
}) {
  const canEditEntries = [
    'maintainers',
    `author:${request.author_user_id}`,
    ...request.editor_user_ids.map((userId) => `editor:${userId}`),
  ]

  return (
    <section className="mt-8">
      <h2 className="text-balance text-lg font-semibold leading-7">
        Collaboration
      </h2>
      <SectionRows className="mt-2">
        <SectionRow
          columns="compact"
          description="Use the CLI to work on the request branch locally."
          icon={<GitBranch className="size-4 text-muted-foreground" />}
          title="Work locally"
        >
          <div className="grid gap-2 text-sm leading-5">
            {request.permissions.can_pull_branch ? (
              <code className="break-all rounded-md bg-muted px-2 py-1 font-mono text-xs">
                scope request join {request.id}
              </code>
            ) : (
              <p className="text-pretty text-sm leading-5 text-muted-foreground">
                {request.permissions.can_push_branch
                  ? 'The request branch has not been pushed yet.'
                  : 'You need edit access before joining this request branch.'}
              </p>
            )}
            <div className="flex flex-wrap gap-1.5">
              <Badge
                variant={
                  request.permissions.can_pull_branch ? 'success' : 'neutral'
                }
              >
                {request.permissions.can_pull_branch ? 'Can pull' : 'Cannot pull'}
              </Badge>
              <Badge
                variant={
                  request.permissions.can_push_branch ? 'success' : 'neutral'
                }
              >
                {request.permissions.can_push_branch ? 'Can push' : 'Cannot push'}
              </Badge>
            </div>
          </div>
        </SectionRow>

        <SectionRow
          columns="compact"
          description="Maintainers can always edit. Public users need to be the author or an editor."
          icon={<UserPlus className="size-4 text-muted-foreground" />}
          title="Can edit"
        >
          <div className="grid gap-3">
            <div className="flex flex-wrap gap-1.5">
              {canEditEntries.map((entry) => (
                <Badge key={entry} variant="outline">
                  {entry}
                </Badge>
              ))}
            </div>

            {request.permissions.can_invite_editor ? (
              <form className="grid gap-3" onSubmit={onAddEditor}>
                <div className="flex flex-col gap-2 sm:flex-row">
                  <input
                    aria-label="Editor user id"
                    className={cn(
                      'h-9 min-w-0 flex-1 rounded-lg border border-input',
                      'bg-background px-3 text-sm outline-none',
                      'placeholder:text-muted-foreground focus-visible:border-ring',
                      'focus-visible:ring-3 focus-visible:ring-ring/50',
                    )}
                    onChange={(event) =>
                      onEditorUserIdChange(event.target.value)
                    }
                    placeholder="Scope user id"
                    value={editorUserId}
                  />
                  <Button
                    disabled={
                      !editorUserId.trim() || pendingAction === 'add-editor'
                    }
                    size="sm"
                    type="submit"
                    variant="secondary"
                  >
                    <UserPlus className="size-3.5" />
                    <span>
                      {pendingAction === 'add-editor' ? 'Adding' : 'Add editor'}
                    </span>
                  </Button>
                </div>
                {errorFor(actionError, 'add-editor') && (
                  <CollaborationError>
                    {errorFor(actionError, 'add-editor')}
                  </CollaborationError>
                )}
              </form>
            ) : null}

            {request.permissions.can_invite_editor &&
            request.editor_user_ids.length > 0 ? (
              <div className="flex flex-wrap gap-1.5">
                {request.editor_user_ids.map((userId) => (
                  <Button
                    disabled={pendingAction === 'remove-editor'}
                    key={userId}
                    onClick={() => onRemoveEditor(userId)}
                    size="sm"
                    type="button"
                    variant="ghost"
                  >
                    <X className="size-3.5" />
                    <span>Remove {userId}</span>
                  </Button>
                ))}
              </div>
            ) : null}
            {errorFor(actionError, 'remove-editor') && (
              <CollaborationError>
                {errorFor(actionError, 'remove-editor')}
              </CollaborationError>
            )}
          </div>
        </SectionRow>
      </SectionRows>
    </section>
  )
}

function CollaborationError({ children }: { children: ReactNode }) {
  return <p className="text-sm leading-5 text-destructive">{children}</p>
}

function errorFor(
  actionError: RequestCollaborationError | null,
  key: RequestCollaborationActionKey,
) {
  return actionError?.key === key ? actionError.message : null
}
