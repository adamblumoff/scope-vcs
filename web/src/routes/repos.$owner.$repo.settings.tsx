import {
  deleteRepoForRequest,
  loadRepoForRequest,
  loadRepoSettingsForRequest,
  parseRepoParams,
  parseUpdateRepoSettingsInput,
  updateRepoSettingsForRequest,
} from '@/api/repos'
import { RepoSettingsPage } from '@/features/repo-detail/repo-settings-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepoSettings = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    const [detail, settings] = await Promise.all([
      loadRepoForRequest(data),
      loadRepoSettingsForRequest(data),
    ])
    return { detail, settings }
  })

const updateRepoSettings = createServerFn({ method: 'POST' })
  .validator(parseUpdateRepoSettingsInput)
  .handler(({ data }) => updateRepoSettingsForRequest(data))

const deleteRepo = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => deleteRepoForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/settings')({
  loader: ({ params }) => loadRepoSettings({ data: params }),
  component: RepoSettingsRoute,
})

function RepoSettingsRoute() {
  const params = Route.useParams()
  const { detail, settings } = Route.useLoaderData()

  return (
    <RepoSettingsPage
      deleteRepo={(data) => deleteRepo({ data })}
      detail={detail}
      initialSettings={settings}
      params={params}
      updateSettings={(data) => updateRepoSettings({ data })}
    />
  )
}
