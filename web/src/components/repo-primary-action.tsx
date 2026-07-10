import type { RepoSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import type { ComponentProps } from 'react'

type RepoActionRoute =
  | '/repos/$owner/$repo'
  | '/repos/$owner/$repo/settings'

type RepoPrimaryAction = {
  label: string
  to: RepoActionRoute
}

type RepoAttentionAction = {
  icon: 'init'
  label: string
  primaryLabel: string
  to: RepoActionRoute
}

type RepoPrimaryActionOptions = {
  includeOpen?: boolean
  requireOwner?: boolean
}

function repoPrimaryAction(
  repo: RepoSummary,
  {
    includeOpen = true,
    requireOwner = false,
  }: RepoPrimaryActionOptions = {},
): RepoPrimaryAction | null {
  if (requireOwner && repo.access.actor !== 'Owner') {
    return null
  }

  const attentionAction = repoAttentionAction(repo)
  if (attentionAction) {
    return {
      label: attentionAction.primaryLabel,
      to: attentionAction.to,
    }
  }

  return includeOpen ? { label: 'Open', to: '/repos/$owner/$repo' } : null
}

function repoAttentionAction(
  repo: RepoSummary,
): RepoAttentionAction | null {
  if (repo.lifecycle_state === 'Unpublished') {
    if (repo.access.actor !== 'Owner') {
      return null
    }

    return {
      icon: 'init',
      label: 'Initialization incomplete',
      primaryLabel: 'Clean up',
      to: '/repos/$owner/$repo/settings',
    }
  }

  return null
}

export function RepoPrimaryActionButton({
  includeOpen,
  repo,
  requireOwner,
  variant = 'secondary',
}: {
  includeOpen?: boolean
  repo: RepoSummary
  requireOwner?: boolean
  variant?: ComponentProps<typeof Button>['variant']
}) {
  const action = repoPrimaryAction(repo, { includeOpen, requireOwner })

  if (!action) {
    return null
  }

  return (
    <Button asChild size="sm" variant={variant}>
      <Link
        params={{ owner: repo.owner_handle, repo: repo.name }}
        to={action.to}
      >
        <ArrowRight className="size-3.5" />
        <span>{action.label}</span>
      </Link>
    </Button>
  )
}
