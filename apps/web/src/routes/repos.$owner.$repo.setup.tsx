import { parseRepoParams } from '@/api/repos'
import {
  loadSetupForRequest,
  loadSetupProgressForRequest,
  regenerateTokenForRequest,
} from '@/api/setup'
import { SetupError, SetupPage } from '@/features/setup/setup-page'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadSetup = createServerFn({ method: 'GET' })
  .validator((input) =>
    parseRepoParams(input, 'Repository setup route is missing owner or repo.'),
  )
  .handler(({ data }) => loadSetupForRequest(data))

const loadSetupProgress = createServerFn({ method: 'POST' })
  .validator((input) =>
    parseRepoParams(input, 'Repository setup route is missing owner or repo.'),
  )
  .handler(({ data }) => loadSetupProgressForRequest(data))

const regenerateToken = createServerFn({ method: 'POST' })
  .validator((input) =>
    parseRepoParams(input, 'Repository setup route is missing owner or repo.'),
  )
  .handler(({ data }) => regenerateTokenForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/setup')({
  loader: async ({ params }) => {
    const state = await loadSetup({ data: params })
    if (state.kind === 'review') {
      throw redirect({
        params,
        to: '/repos/$owner/$repo/review',
      })
    }

    return state.setup
  },
  errorComponent: SetupError,
  component: SetupRoute,
})

function SetupRoute() {
  const params = Route.useParams()

  return (
    <SetupPage
      initialSetup={Route.useLoaderData()}
      loadProgress={(data) => loadSetupProgress({ data })}
      params={params}
      regenerateToken={(data) => regenerateToken({ data })}
    />
  )
}
