import { loadHomeForRequest } from '@/api/repos'
import { HomePage } from '@/features/home/home-page'
import { MarketingLandingPage } from '@/features/marketing/marketing-landing-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadIndex = createServerFn({ method: 'GET' }).handler(async () => {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const { isAuthenticated } = await auth()

  if (!isAuthenticated) {
    return { kind: 'marketing' } as const
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
    return <MarketingLandingPage />
  }

  return <HomePage home={state.home} />
}
