import { ApplicationPendingShell } from '@/components/pending-surface'
import { useParams } from '@tanstack/react-router'

export function RepositoryRoutePending() {
  const repository = useParams({ from: '/$owner/$repo' })

  return (
    <ApplicationPendingShell
      label={`Loading ${repository.owner}/${repository.repo}`}
      repository={repository}
    />
  )
}
