import { HttpError } from '@/api/client'
import { loadOwnerProfileForRequest } from '@/api/profile'
import { OwnerProfileError } from '@/features/home/owner-profile-error'
import { OwnerProfileNotFound } from '@/features/home/owner-profile-not-found'
import { OwnerProfileRoute } from '@/features/home/owner-profile-route'
import { createFileRoute, notFound } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadOwnerProfile = createServerFn({ method: 'GET' })
  .validator((input: { owner: string }) => input)
  .handler(async ({ data }) => {
    try {
      return await loadOwnerProfileForRequest(data.owner)
    } catch (error) {
      if (error instanceof HttpError && error.status === 404) {
        throw notFound()
      }
      throw error
    }
  })

export const Route = createFileRoute('/$owner/')({
  loader: ({ params }) => loadOwnerProfile({ data: params }),
  errorComponent: OwnerProfileError,
  notFoundComponent: OwnerProfileNotFound,
  component: OwnerProfileRoute,
})
