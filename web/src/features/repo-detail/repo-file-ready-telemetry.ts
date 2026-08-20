import type { RepoParams } from '@/api/types'
import { createServerFn } from '@tanstack/react-start'
import { useEffect, useRef } from 'react'

type RepoFileReadyTiming = RepoParams & {
  duration_ms: number
  navigation_kind: 'client' | 'document'
  path: string
}

const reportRepoFileReady = createServerFn({ method: 'POST' })
  .validator(parseRepoFileReadyTiming)
  .handler(({ data }) => {
    console.info(JSON.stringify({ event: 'repo_file_ready', ...data }))
  })

export function useRepoFileReadyTiming({
  identity,
  owner,
  path,
  ready,
  repo,
}: {
  identity: string | null
  owner: string
  path: string | null
  ready: boolean
  repo: string
}) {
  const firstIdentity = useRef(true)
  const measurement = useRef<{
    identity: string | null
    navigationKind: 'client' | 'document'
    startedAt: number
  }>({ identity: null, navigationKind: 'document', startedAt: 0 })
  const reportedIdentity = useRef<string | null>(null)

  if (
    typeof performance !== 'undefined' &&
    measurement.current.identity !== identity
  ) {
    measurement.current = {
      identity,
      navigationKind: firstIdentity.current ? 'document' : 'client',
      startedAt: firstIdentity.current ? 0 : performance.now(),
    }
    firstIdentity.current = false
  }

  useEffect(() => {
    if (!identity || !path || !ready || reportedIdentity.current === identity) {
      return
    }
    reportedIdentity.current = identity
    const frame = requestAnimationFrame(() => {
      const current = measurement.current
      if (current.identity !== identity) return
      void reportRepoFileReady({
        data: {
          duration_ms: Math.round(performance.now() - current.startedAt),
          navigation_kind: current.navigationKind,
          owner,
          path,
          repo,
        },
      }).catch(() => undefined)
    })
    return () => cancelAnimationFrame(frame)
  }, [identity, owner, path, ready, repo])
}

function parseRepoFileReadyTiming(input: RepoFileReadyTiming) {
  if (
    !input ||
    typeof input.owner !== 'string' ||
    typeof input.repo !== 'string' ||
    typeof input.path !== 'string' ||
    !Number.isFinite(input.duration_ms) ||
    input.duration_ms < 0 ||
    input.duration_ms > 300_000 ||
    (input.navigation_kind !== 'client' && input.navigation_kind !== 'document')
  ) {
    throw new Error('invalid repository file timing')
  }
  return input
}
