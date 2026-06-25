import { loadHomeForRequest } from '@/api/repos'
import { HomePage } from '@/features/home/home-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadHome = createServerFn({ method: 'GET' }).handler(loadHomeForRequest)

export const Route = createFileRoute('/')({
  loader: () => loadHome(),
  component: HomeRoute,
})

function HomeRoute() {
  return <HomePage home={Route.useLoaderData()} />
}
