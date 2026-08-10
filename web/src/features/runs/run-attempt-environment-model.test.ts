import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import type { RepoRunCache } from '@/api/types'
import {
  cacheExplanation,
  cacheStateClass,
  cacheSummaryLabel,
  cacheTimingLabel,
  pinnedImageLabel,
  summarizeAttemptCaches,
} from './run-attempt-environment-model'

const caches: RepoRunCache[] = [
  {
    name: 'cargo',
    path: '/scope/cache/cargo',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'a'.repeat(64),
      preparation: { kind: 'warm' },
      prepare_ms: 200,
      final_state: 'ready',
      finalize_ms: 100,
    },
  },
  {
    name: 'target',
    path: '/workspace/target',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'b'.repeat(64),
      preparation: { kind: 'cold', reason: 'metadata-missing' },
      prepare_ms: 1_100,
      final_state: 'pending',
      finalize_ms: null,
    },
  },
  {
    name: 'playwright',
    path: '/root/.cache/ms-playwright',
    observation: null,
  },
]

describe('run attempt environment model', () => {
  it('summarizes only reported preparation facts', () => {
    assert.deepEqual(summarizeAttemptCaches(caches), {
      cold: 1,
      prepareMs: 1_100,
      unavailable: 1,
      warm: 1,
    })
    assert.equal(
      cacheSummaryLabel(caches),
      '1 warm · 1 cold · 1 not reported · prepared in 1.1s',
    )
  })

  it('keeps missing metadata distinct from a missing report', () => {
    assert.equal(
      cacheExplanation(caches[1]!),
      'No reusable entry for this identity · pending',
    )
    assert.equal(
      cacheExplanation(caches[2]!),
      'Cache facts were not reported for this attempt.',
    )
    assert.equal(cacheTimingLabel(caches[2]!), 'unavailable')
    assert.equal(cacheStateClass(caches[0]!), 'text-emerald-600')
    assert.equal(cacheStateClass(caches[1]!), 'text-amber-600')
    assert.equal(cacheStateClass(caches[2]!), 'text-muted-foreground')
    assert.equal(
      cacheSummaryLabel([caches[2]!]),
      '1 not reported',
    )
  })

  it('formats immutable image identity without the registry noise', () => {
    assert.equal(
      pinnedImageLabel(`registry/scope@sha256:${'c'.repeat(64)}`),
      `sha256:${'c'.repeat(12)}`,
    )
    assert.equal(pinnedImageLabel(null), 'Image not pinned yet')
  })
})
