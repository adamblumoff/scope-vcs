import type {
  CreateRepoInviteInput,
  CreateRepoInviteResponse,
  RepoCollaboration,
  RepoInvite,
  RepoMember,
  RepoMemberPermissions,
  RepoParams,
  RepoSummary,
  UpdateRepoMemberInput,
} from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import {
  Eye,
  LoaderCircle,
  MailPlus,
  ShieldCheck,
  Trash2,
  Users,
} from 'lucide-react'
import { useReducer, useState, type FormEvent } from 'react'

const defaultPermissions: RepoMemberPermissions = {
  can_apply_changes: false,
  can_change_file_visibility: false,
  can_push: false,
}

const permissionLabels = [
  {
    description: 'Allows Git pushes to this repository.',
    key: 'can_push',
    label: 'Push changes',
  },
] as const

type PermissionKey = (typeof permissionLabels)[number]['key']

type InviteMemberFormState = {
  email: string
  error: string | null
  inviteUrl: string | null
  pending: boolean
  permissions: RepoMemberPermissions
}

type InviteMemberFormAction =
  | { email: string; type: 'emailChanged' }
  | { permissions: RepoMemberPermissions; type: 'permissionsChanged' }
  | { type: 'submitStarted' }
  | { inviteUrl: string; type: 'submitSucceeded' }
  | { message: string; type: 'submitFailed' }

const initialInviteMemberFormState: InviteMemberFormState = {
  email: '',
  error: null,
  inviteUrl: null,
  pending: false,
  permissions: defaultPermissions,
}

function inviteMemberFormReducer(
  state: InviteMemberFormState,
  action: InviteMemberFormAction,
): InviteMemberFormState {
  switch (action.type) {
    case 'emailChanged':
      return { ...state, email: action.email }
    case 'permissionsChanged':
      return { ...state, permissions: action.permissions }
    case 'submitStarted':
      return { ...state, error: null, inviteUrl: null, pending: true }
    case 'submitSucceeded':
      return {
        ...state,
        email: '',
        inviteUrl: action.inviteUrl,
        pending: false,
        permissions: defaultPermissions,
      }
    case 'submitFailed':
      return { ...state, error: action.message, pending: false }
  }
}

export function MemberAccessSections({
  repo,
}: {
  repo: RepoSummary
}) {
  return (
    <SectionRows>
      <SectionRow
        description="These permissions are assigned by the repository owner."
        icon={<ShieldCheck className="size-4" />}
        title="Your access"
      >
        <div className="space-y-3 text-sm">
          <AlwaysOnPrivateRead />
          <PermissionSummary permissions={repo.access} />
        </div>
      </SectionRow>
    </SectionRows>
  )
}

export function RepositoryMembersSection({
  collaboration,
  createInvite,
  deleteInvite,
  deleteMember,
  params,
  repo,
  updateMember,
}: {
  collaboration: RepoCollaboration
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<CreateRepoInviteResponse>
  deleteInvite: (inviteId: string) => Promise<RepoInvite>
  deleteMember: (memberUserId: string) => Promise<RepoMember>
  params: RepoParams
  repo: RepoSummary
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepoMember>
}) {
  const canInvite = repo.lifecycle_state === 'Published'
  const pendingInvites = collaboration.invites.filter(
    (invite) => invite.state === 'Pending',
  )

  return (
    <SectionRows>
      <SectionRow
        description={
          canInvite
            ? 'Invite members by email and assign only the extra actions they need.'
            : 'Members can be invited after the first Scope push is applied.'
        }
        icon={<MailPlus className="size-4" />}
        title="Invite member"
      >
        <InviteMemberForm
          canInvite={canInvite}
          createInvite={(input) =>
            createInvite({
              ...input,
              owner: params.owner,
              repo: params.repo,
            })
          }
        />
      </SectionRow>

      <SectionRow
        description="Members always read private files. Toggles only control repository actions."
        icon={<Users className="size-4" />}
        title="Members"
      >
        <MemberList
          deleteMember={deleteMember}
          members={collaboration.members}
          params={params}
          updateMember={updateMember}
        />
      </SectionRow>

      {pendingInvites.length > 0 && (
        <SectionRow
          description="Pending email invites are unique per repository and email."
          icon={<MailPlus className="size-4" />}
          title="Pending invites"
        >
          <InviteList deleteInvite={deleteInvite} invites={pendingInvites} />
        </SectionRow>
      )}
    </SectionRows>
  )
}

function InviteMemberForm({
  canInvite,
  createInvite,
}: {
  canInvite: boolean
  createInvite: (
    input: Omit<CreateRepoInviteInput, 'owner' | 'repo'>,
  ) => Promise<CreateRepoInviteResponse>
}) {
  const [state, dispatch] = useReducer(
    inviteMemberFormReducer,
    initialInviteMemberFormState,
  )

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!canInvite || state.pending) {
      return
    }

    dispatch({ type: 'submitStarted' })
    try {
      const response = await createInvite({
        email: state.email,
        permissions: state.permissions,
      })
      dispatch({ inviteUrl: response.invite_url, type: 'submitSucceeded' })
    } catch (error) {
      dispatch({
        message: error instanceof Error ? error.message : 'invite failed',
        type: 'submitFailed',
      })
    }
  }

  return (
    <form className="space-y-4" onSubmit={(event) => void submit(event)}>
      <div className="flex flex-col gap-2 sm:flex-row">
        <Input
          aria-label="Member email"
          disabled={!canInvite || state.pending}
          onChange={(event) =>
            dispatch({ email: event.target.value, type: 'emailChanged' })
          }
          placeholder="teammate@example.com"
          type="email"
          value={state.email}
        />
        <Button
          disabled={!canInvite || state.pending || !state.email.trim()}
          type="submit"
        >
          {state.pending ? (
            <LoaderCircle className="size-3.5 animate-spin" />
          ) : (
            <MailPlus className="size-3.5" />
          )}
          <span>Invite</span>
        </Button>
      </div>

      <div className="rounded-lg border border-amber-400/50 bg-amber-100 px-3 py-2 text-sm leading-5 text-amber-900 dark:bg-amber-400/10 dark:text-amber-100">
        Members always read private files once they accept. This toggle grants
        repository push access only.
      </div>

      <PermissionEditor
        disabled={!canInvite || state.pending}
        onChange={(permissions) =>
          dispatch({ permissions, type: 'permissionsChanged' })
        }
        permissions={state.permissions}
      />

      {state.error && <p className="text-sm text-destructive">{state.error}</p>}
      {state.inviteUrl && (
        <div className="space-y-2">
          <p className="text-sm text-muted-foreground">
            Invite created. Share this link with the invitee.
          </p>
          <CopyableCodeBlock
            copyLabel="Copy invite link"
            value={state.inviteUrl}
          />
        </div>
      )}
    </form>
  )
}

function MemberList({
  deleteMember,
  members,
  params,
  updateMember,
}: {
  deleteMember: (memberUserId: string) => Promise<RepoMember>
  members: RepoMember[]
  params: RepoParams
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepoMember>
}) {
  const [error, setError] = useState<string | null>(null)
  const [pendingKey, setPendingKey] = useState<string | null>(null)

  if (members.length === 0) {
    return (
      <p className="text-sm leading-5 text-muted-foreground">
        No members have accepted an invite yet.
      </p>
    )
  }

  async function update(
    member: RepoMember,
    permissions: RepoMemberPermissions,
  ) {
    setError(null)
    setPendingKey(member.user_id)
    try {
      await updateMember({
        ...params,
        member_user_id: member.user_id,
        permissions,
      })
    } catch (error) {
      setError(error instanceof Error ? error.message : 'member update failed')
    } finally {
      setPendingKey(null)
    }
  }

  async function remove(member: RepoMember) {
    setError(null)
    setPendingKey(member.user_id)
    try {
      await deleteMember(member.user_id)
    } catch (error) {
      setError(error instanceof Error ? error.message : 'member removal failed')
    } finally {
      setPendingKey(null)
    }
  }

  return (
    <div className="space-y-3">
      <ul className="divide-y divide-border">
        {members.map((member) => {
          const pending = pendingKey === member.user_id
          return (
            <li className="space-y-3 py-3 first:pt-0" key={member.user_id}>
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium leading-5">
                    @{member.handle}
                  </div>
                  <div className="truncate text-sm leading-5 text-muted-foreground">
                    {member.email}
                  </div>
                </div>
                <Button
                  disabled={pending}
                  onClick={() => void remove(member)}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  {pending ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="size-3.5" />
                  )}
                  <span>Remove</span>
                </Button>
              </div>
              <AlwaysOnPrivateRead />
              <PermissionEditor
                disabled={pending}
                onChange={(permissions) => void update(member, permissions)}
                permissions={member.permissions}
              />
            </li>
          )
        })}
      </ul>
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}

function InviteList({
  deleteInvite,
  invites,
}: {
  deleteInvite: (inviteId: string) => Promise<RepoInvite>
  invites: RepoInvite[]
}) {
  const [error, setError] = useState<string | null>(null)
  const [pendingId, setPendingId] = useState<string | null>(null)

  async function revoke(invite: RepoInvite) {
    setError(null)
    setPendingId(invite.id)
    try {
      await deleteInvite(invite.id)
    } catch (error) {
      setError(error instanceof Error ? error.message : 'invite revoke failed')
    } finally {
      setPendingId(null)
    }
  }

  return (
    <div className="space-y-3">
      <ul className="divide-y divide-border">
        {invites.map((invite) => {
          const pending = pendingId === invite.id
          return (
            <li
              className="flex flex-col gap-2 py-3 text-sm first:pt-0 sm:flex-row sm:items-center sm:justify-between"
              key={invite.id}
            >
              <div className="min-w-0">
                <div className="truncate font-medium leading-5">
                  {invite.invited_email}
                </div>
                <div className="leading-5 text-muted-foreground">
                  {permissionSummaryText(invite.permissions)}
                </div>
              </div>
              <div className="flex items-center gap-2">
                <Badge variant="warning">{invite.state}</Badge>
                <Button
                  disabled={pending}
                  onClick={() => void revoke(invite)}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  {pending ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="size-3.5" />
                  )}
                  <span>Revoke</span>
                </Button>
              </div>
            </li>
          )
        })}
      </ul>
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}

function PermissionEditor({
  disabled,
  onChange,
  permissions,
}: {
  disabled?: boolean
  onChange: (permissions: RepoMemberPermissions) => void
  permissions: RepoMemberPermissions
}) {
  return (
    <div className="space-y-2">
      {permissionLabels.map((permission) => (
        <label
          className="flex items-start justify-between gap-4 text-sm"
          key={permission.key}
        >
          <span className="min-w-0">
            <span className="block font-medium leading-5">
              {permission.label}
            </span>
            <span className="block leading-5 text-muted-foreground">
              {permission.description}
            </span>
          </span>
          <Switch
            checked={permissions[permission.key]}
            disabled={disabled}
            onCheckedChange={(checked) =>
              onChange({ ...permissions, [permission.key]: checked })
            }
            type="button"
          />
        </label>
      ))}
    </div>
  )
}

function PermissionSummary({
  permissions,
}: {
  permissions: RepoMemberPermissions
}) {
  return (
    <div className="space-y-2">
      {permissionLabels.map((permission) => (
        <ReadOnlyPermission
          enabled={permissions[permission.key]}
          key={permission.key}
          label={permission.label}
        />
      ))}
    </div>
  )
}

function ReadOnlyPermission({
  enabled,
  label,
}: {
  enabled: boolean
  label: string
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span>{label}</span>
      <Badge variant={enabled ? 'success' : 'neutral'}>
        {enabled ? 'On' : 'Off'}
      </Badge>
    </div>
  )
}

function AlwaysOnPrivateRead() {
  return (
    <div className="flex items-center justify-between gap-3 text-sm">
      <span className="inline-flex items-center gap-2">
        <Eye className="size-3.5 text-muted-foreground" />
        <span>Read private files</span>
      </span>
      <Badge variant="success">Always on</Badge>
    </div>
  )
}

function permissionSummaryText(permissions: RepoMemberPermissions) {
  const enabled = permissionLabels.reduce<string[]>((labels, permission) => {
    if (permissions[permission.key]) {
      labels.push(permission.label.toLowerCase())
    }
    return labels
  }, [])

  return enabled.length === 0 ? 'No extra actions' : enabled.join(', ')
}
