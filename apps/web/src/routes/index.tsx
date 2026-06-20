import {
  createRepoForRequest,
  deleteRepoForRequest,
  loadHomeForRequest,
  parseCreateRepoInput,
  parseDeleteRepoInput,
} from '@/api/repos'
import { HomePage } from '@/features/home/home-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadHome = createServerFn({ method: 'GET' }).handler(loadHomeForRequest)

const createRepo = createServerFn({ method: 'POST' })
  .validator(parseCreateRepoInput)
  .handler(({ data }) => createRepoForRequest(data))

const deleteRepo = createServerFn({ method: 'POST' })
  .validator(parseDeleteRepoInput)
  .handler(({ data }) => deleteRepoForRequest(data))

export const Route = createFileRoute('/')({
  loader: () => loadHome(),
  component: HomeRoute,
})

function HomeRoute() {
  return (
    <HomePage
      createRepo={(data) => createRepo({ data })}
      deleteRepo={(data) => deleteRepo({ data })}
      home={Route.useLoaderData()}
    />
  )
}
