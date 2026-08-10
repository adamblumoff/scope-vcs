import { buildCliInstallCommands } from '@/api/cli-install'
import { loadAuthenticatedAccountForRequest } from '@/api/profile'
import { MarketingLandingPage } from '@/features/marketing/marketing-landing-page'
import { detectCliPlatform } from '@/lib/cli-platform'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadIndex = createServerFn({ method: 'GET' }).handler(async () => {
  const [{ auth }, { getRequestHeader }] = await Promise.all([
    import('@clerk/tanstack-react-start/server'),
    import('@tanstack/react-start/server'),
  ])
  const { isAuthenticated } = await auth()

  if (!isAuthenticated) {
    const platformHeader = getRequestHeader('sec-ch-ua-platform')
      ?? getRequestHeader('user-agent')

    return {
      cliInstallCommands: buildCliInstallCommands(),
      initialCliPlatform: detectCliPlatform(platformHeader),
      kind: 'marketing',
    } as const
  }

  const account = await loadAuthenticatedAccountForRequest()
  const handle = account.user?.handle
  if (!handle) {
    throw new Error('Signed-in account is missing a Scope handle.')
  }
  throw redirect({ params: { owner: handle }, to: '/$owner' })
})

export const Route = createFileRoute('/')({
  loader: () => loadIndex(),
  component: IndexRoute,
})

function IndexRoute() {
  const state = Route.useLoaderData()

  if (state.kind === 'marketing') {
    return (
      <MarketingLandingPage
        cliInstallCommands={state.cliInstallCommands}
        initialCliPlatform={state.initialCliPlatform}
      />
    )
  }
}
