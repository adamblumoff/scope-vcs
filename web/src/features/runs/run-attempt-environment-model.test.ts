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
      preparation: { kind: 'exact' },
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
    name: 'cargo-target',
    path: '/scope/cache/cargo-target',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'c'.repeat(64),
      preparation: { kind: 'compatible' },
      prepare_ms: 900,
      final_state: 'ready',
      finalize_ms: 50,
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
      warm: 2,
    })
    assert.equal(
      cacheSummaryLabel(caches),
      '2 warm · 1 cold · 1 not reported · prepared in 1.1s',
    )
  })

  it('keeps missing metadata distinct from a missing report', () => {
    assert.equal(
      cacheExplanation(caches[1]!),
      'No reusable entry for this identity · pending',
    )
    assert.equal(
      cacheExplanation(caches[3]!),
      'Cache facts were not reported for this attempt.',
    )
    assert.equal(cacheExplanation(caches[0]!), 'Exact entry found · ready')
    assert.equal(
      cacheExplanation(caches[2]!),
      'Compatible fallback found · ready',
    )
    assert.equal(cacheTimingLabel(caches[3]!), 'unavailable')
    assert.equal(cacheStateClass(caches[0]!), 'text-success')
    assert.equal(cacheStateClass(caches[1]!), 'text-warning')
    assert.equal(cacheStateClass(caches[2]!), 'text-success')
    assert.equal(cacheStateClass(caches[3]!), 'text-muted-foreground')
    assert.equal(
      cacheSummaryLabel([caches[3]!]),
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
