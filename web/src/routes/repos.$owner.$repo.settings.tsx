import {
  createRepoInviteForRequest,
  deleteRepoMemberForRequest,
  deleteRepoForRequest,
  loadRepoCollaborationForRequest,
  loadRepoForRequest,
  loadRepoSettingsForRequest,
  parseCreateRepoInviteInput,
  parseDeleteRepoMemberInput,
  parseRepoParams,
  parseUpdateRepoMemberInput,
  parseUpdateRepoSettingsInput,
  updateRepoMemberForRequest,
  updateRepoSettingsForRequest,
} from '@/api/repos'
import { RepoSettingsPage } from '@/features/repo-detail/repo-settings-page'
import { useRepoLiveRefresh } from '@/features/repo-detail/repo-live-refresh'
import { createFileRoute, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const loadRepoSettings = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    const detail = await loadRepoForRequest(data)
    const [settings, collaboration] = await Promise.all([
      detail.repo.access.can_update_repo_settings
        ? loadRepoSettingsForRequest(data)
        : Promise.resolve(null),
      detail.repo.access.can_manage_members
        ? loadRepoCollaborationForRequest(data)
        : Promise.resolve(null),
    ])
    return { collaboration, detail, settings }
  })

const updateRepoSettings = createServerFn({ method: 'POST' })
  .validator(parseUpdateRepoSettingsInput)
  .handler(({ data }) => updateRepoSettingsForRequest(data))

const deleteRepo = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => deleteRepoForRequest(data))

const createRepoInvite = createServerFn({ method: 'POST' })
  .validator(parseCreateRepoInviteInput)
  .handler(({ data }) => createRepoInviteForRequest(data))

const updateRepoMember = createServerFn({ method: 'POST' })
  .validator(parseUpdateRepoMemberInput)
  .handler(({ data }) => updateRepoMemberForRequest(data))

const deleteRepoMember = createServerFn({ method: 'POST' })
  .validator(parseDeleteRepoMemberInput)
  .handler(({ data }) => deleteRepoMemberForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/settings')({
  loader: ({ params }) => loadRepoSettings({ data: params }),
  component: RepoSettingsRoute,
})

function RepoSettingsRoute() {
  const params = Route.useParams()
  const { collaboration, detail, settings } = Route.useLoaderData()
  const router = useRouter()
  const invalidate = useCallback(() => router.invalidate(), [router])
  useRepoLiveRefresh(detail.live, invalidate)

  return (
    <RepoSettingsPage
      createInvite={(data) => createRepoInvite({ data })}
      deleteRepo={(data) => deleteRepo({ data })}
      deleteMember={(data) => deleteRepoMember({ data })}
      detail={detail}
      initialCollaboration={collaboration}
      initialSettings={settings}
      params={params}
      updateMember={(data) => updateRepoMember({ data })}
      updateSettings={(data) => updateRepoSettings({ data })}
    />
  )
}
