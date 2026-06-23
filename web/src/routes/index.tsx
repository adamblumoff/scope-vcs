import {
  createRepoForRequest,
  loadHomeForRequest,
  parseCreateRepoInput,
} from '@/api/repos'
import { HomePage } from '@/features/home/home-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadHome = createServerFn({ method: 'GET' }).handler(loadHomeForRequest)

const createRepo = createServerFn({ method: 'POST' })
  .validator(parseCreateRepoInput)
  .handler(({ data }) => createRepoForRequest(data))

export const Route = createFileRoute('/')({
  loader: () => loadHome(),
  component: HomeRoute,
})

function HomeRoute() {
  return (
    <HomePage
      createRepo={(data) => createRepo({ data })}
      home={Route.useLoaderData()}
    />
  )
}
