import { loadHomeForRequest } from '@/api/repos'
import { HomePage } from '@/features/home/home-page'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const requireHomeAuth = createServerFn({ method: 'GET' }).handler(async () => {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const { isAuthenticated } = await auth()
  if (!isAuthenticated) {
    throw redirect({ params: { _splat: '' }, to: '/sign-in/$' })
  }
})

const loadHome = createServerFn({ method: 'GET' }).handler(loadHomeForRequest)

export const Route = createFileRoute('/')({
  beforeLoad: () => requireHomeAuth(),
  loader: () => loadHome(),
  component: HomeRoute,
})

function HomeRoute() {
  return <HomePage home={Route.useLoaderData()} />
}
