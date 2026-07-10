import {
  createRepoInviteForRequest,
  deleteRepoInviteForRequest,
  deleteRepoMemberForRequest,
  deleteRepoForRequest,
  loadRepoCollaborationForRequest,
  parseCreateRepoInviteInput,
  parseDeleteRepoInviteInput,
  parseDeleteRepoMemberInput,
  parseRepoParams,
  parseUpdateRepoMemberInput,
  updateRepoMemberForRequest,
} from '@/api/repos'
import { HttpError } from '@/api/client'
import { RepoSettingsPage } from '@/features/repo-detail/repo-settings-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepoSettings = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    try {
      return await loadRepoCollaborationForRequest(data)
    } catch (error) {
      if (error instanceof HttpError && [403, 404].includes(error.status)) {
        return null
      }
      throw error
    }
  })

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

const deleteRepoInvite = createServerFn({ method: 'POST' })
  .validator(parseDeleteRepoInviteInput)
  .handler(({ data }) => deleteRepoInviteForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/settings')({
  loader: ({ params }) => loadRepoSettings({ data: params }),
  component: RepoSettingsRoute,
})

function RepoSettingsRoute() {
  const params = Route.useParams()
  const collaboration = Route.useLoaderData()
  return (
    <RepoSettingsPage
      createInvite={(data) => createRepoInvite({ data })}
      deleteInvite={(data) => deleteRepoInvite({ data })}
      deleteRepo={(data) => deleteRepo({ data })}
      deleteMember={(data) => deleteRepoMember({ data })}
      initialCollaboration={collaboration}
      params={params}
      updateMember={(data) => updateRepoMember({ data })}
    />
  )
}
