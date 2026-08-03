import { OwnerProfilePage } from '@/features/home/owner-profile-page'
import { useLoaderData } from '@tanstack/react-router'

export function OwnerProfileRoute() {
  const state = useLoaderData({ from: '/$owner/' })
  return <OwnerProfilePage state={state} />
}
