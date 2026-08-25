import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import type { RepoRunCache } from '@/api/types'
import {
  cacheExplanation,
  cachePreparationDetail,
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
      key_ms: 10,
      metadata_ms: 20,
      size_bytes: 512 * 1_024 * 1_024,
      download_verify_ms: 80,
      sync_ms: 40,
      extraction_ms: 50,
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
      key_ms: 100,
      metadata_ms: 1_000,
      size_bytes: 0,
      download_verify_ms: 0,
      sync_ms: 0,
      extraction_ms: 0,
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
      key_ms: 100,
      metadata_ms: 100,
      size_bytes: 1_024 ** 3,
      download_verify_ms: 300,
      sync_ms: 100,
      extraction_ms: 300,
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
      unavailable: 1,
      warm: 2,
    })
    assert.equal(
      cacheSummaryLabel(caches, {
        authorization_ms: 75,
        wall_ms: 2_200,
      }),
      '2 warm · 1 cold · 1 not reported · setup in 2.2s · authorized in 75ms',
    )
    assert.equal(
      cacheSummaryLabel(caches, null),
      '2 warm · 1 cold · 1 not reported',
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
      cacheSummaryLabel([caches[3]!], null),
      '1 not reported',
    )
  })

  it('shows bytes and every preparation phase without inventing a total', () => {
    assert.equal(
      cachePreparationDetail(caches[0]!),
      '512.0 MiB compressed · key 10ms · metadata 20ms · download + verify 80ms · sync 40ms · extract 50ms',
    )
    assert.equal(
      cachePreparationDetail(caches[1]!),
      '0 B compressed · key 100ms · metadata 1.0s · download + verify 0ms · sync 0ms · extract 0ms',
    )
    assert.equal(cacheTimingLabel(caches[0]!), 'total 200ms · finalize 100ms')
    assert.equal(
      cachePreparationDetail(caches[2]!),
      '1.00 GiB compressed · key 100ms · metadata 100ms · download + verify 300ms · sync 100ms · extract 300ms',
    )
    assert.equal(cachePreparationDetail(caches[3]!), null)
  })

  it('formats immutable image identity without the registry noise', () => {
    assert.equal(
      pinnedImageLabel(`registry/scope@sha256:${'c'.repeat(64)}`),
      `sha256:${'c'.repeat(12)}`,
    )
    assert.equal(pinnedImageLabel(null), 'Image not pinned yet')
  })
})
