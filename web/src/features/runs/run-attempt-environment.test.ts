import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { RepoRunCache } from '@/api/types'
import { RunAttemptEnvironment } from './run-attempt-environment'

const image = `registry/scope@sha256:${'c'.repeat(64)}`
const caches: RepoRunCache[] = [
  {
    name: 'cargo',
    path: '/scope/cache/cargo',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'a'.repeat(64),
      preparation: { kind: 'exact' },
      key_ms: 2,
      metadata_ms: 3,
      size_bytes: 1_024 ** 2,
      download_verify_ms: 4,
      sync_ms: 1,
      extraction_ms: 2,
      prepare_ms: 12,
      final_state: 'ready',
      finalize_ms: 8,
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
      key_ms: 1,
      metadata_ms: 2,
      size_bytes: 0,
      download_verify_ms: 0,
      sync_ms: 0,
      extraction_ms: 0,
      prepare_ms: 3,
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

describe('run attempt environment', () => {
  it('renders compact inline facts without presenting them as cards', () => {
    const html = renderToStaticMarkup(createElement(RunAttemptEnvironment, {
      caches,
      cacheSetup: {
        authorization_ms: 3,
        wall_ms: 20,
      },
      pinnedContainerImage: image,
    }))

    assert.match(html, /aria-label="Execution environment"/)
    assert.match(html, /Environment/)
    assert.match(html, /1 warm · 1 cold · 1 not reported · setup in 20ms · authorized in 3ms/)
    assert.match(html, /No reusable entry for this identity/)
    assert.match(
      html,
      /1.0 MiB compressed · key 2ms · metadata 3ms · download \+ verify 4ms · sync 1ms · extract 2ms/,
    )
    assert.match(
      html,
      /0 B compressed · key 1ms · metadata 2ms · download \+ verify 0ms · sync 0ms · extract 0ms/,
    )
    assert.match(html, /total 12ms · finalize 8ms/)
    assert.match(html, /Cache facts were not reported for this attempt/)
    assert.match(html, new RegExp(`title="${image}"`))
    assert.doesNotMatch(html, /rounded|shadow/)
  })
})
