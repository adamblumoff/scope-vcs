import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  activeRepoSection,
  REPO_SECTIONS,
  repoSectionsForActor,
} from './repo-section-model'

describe('repository sections', () => {
  it('orders Runs after Requests for owners and members', () => {
    const expected = ['Code', 'Requests', 'Runs', 'History', 'Settings']
    assert.deepEqual(
      repoSectionsForActor('Owner').map(({ label }) => label),
      expected,
    )
    assert.deepEqual(
      repoSectionsForActor('Member').map(({ label }) => label),
      expected,
    )
  })

  it('hides Runs and Settings from public actors', () => {
    assert.deepEqual(
      repoSectionsForActor('Public').map(({ label }) => label),
      ['Code', 'Requests', 'History'],
    )
  })

  it('recognizes every sibling route before falling back to Code', () => {
    for (const section of REPO_SECTIONS.filter(({ key }) => key !== 'code')) {
      assert.equal(
        activeRepoSection((route) => route === section.to),
        section.key,
      )
    }
    assert.equal(activeRepoSection(() => false), 'code')
  })
})
