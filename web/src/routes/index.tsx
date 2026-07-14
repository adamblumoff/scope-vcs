import { buildCliInstallCommands } from '@/api/cli-install'
import { loadHomeForRequest } from '@/api/repos'
import { HomePage } from '@/features/home/home-page'
import { MarketingLandingPage } from '@/features/marketing/marketing-landing-page'
import { detectCliPlatform } from '@/lib/cli-platform'
import { createFileRoute } from '@tanstack/react-router'
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

  return {
    home: await loadHomeForRequest(),
    kind: 'home',
  } as const
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

  return <HomePage home={state.home} />
}
