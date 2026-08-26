import assert from 'node:assert/strict'
import test from 'node:test'
import { compactDiscussionSummary } from './discussion-preview-text'

test('missing and empty bodies fall back', () => {
  assert.equal(compactDiscussionSummary(null), 'Update')
  assert.equal(compactDiscussionSummary(''), 'Update')
})

test('whitespace only body has nothing to preview', () => {
  assert.equal(compactDiscussionSummary(' \n\t\n   '), 'Untitled discussion')
})

test('heading markers are dropped', () => {
  assert.equal(
    compactDiscussionSummary('\n## Cache invalidation\nMore'),
    'Cache invalidation',
  )
  assert.equal(compactDiscussionSummary('###### Deep'), 'Deep')
})

test('inline code keeps its text without backticks', () => {
  assert.equal(
    compactDiscussionSummary('the name `retryDelay` does not state the unit'),
    'the name retryDelay does not state the unit',
  )
  assert.equal(compactDiscussionSummary('use ``a ` b`` here'), 'use a b here')
})

test('code fences are skipped and their contents kept', () => {
  assert.equal(
    compactDiscussionSummary('```ts\nconst retryDelay = 500\n```'),
    'const retryDelay = 500',
  )
  assert.equal(
    compactDiscussionSummary('~~~\nplain fence body\n~~~'),
    'plain fence body',
  )
})

test('emphasis markers are removed', () => {
  assert.equal(compactDiscussionSummary('**bold** start'), 'bold start')
  assert.equal(compactDiscussionSummary('__bold__ start'), 'bold start')
  assert.equal(compactDiscussionSummary('*italic* start'), 'italic start')
  assert.equal(compactDiscussionSummary('_italic_ start'), 'italic start')
  assert.equal(compactDiscussionSummary('~~gone~~ now'), 'gone now')
  assert.equal(
    compactDiscussionSummary('**bold _nested_ text**'),
    'bold nested text',
  )
})

test('intraword underscores survive', () => {
  assert.equal(
    compactDiscussionSummary('rename retry_delay_ms please'),
    'rename retry_delay_ms please',
  )
})

test('links keep their text', () => {
  assert.equal(
    compactDiscussionSummary('see [the docs](https://example.com/a_b) first'),
    'see the docs first',
  )
})

test('images are dropped entirely', () => {
  assert.equal(
    compactDiscussionSummary('before ![alt text](img.png) after'),
    'before after',
  )
  assert.equal(
    compactDiscussionSummary('before ![](img.png) after'),
    'before after',
  )
})

test('an image only body leaves no stray brackets', () => {
  assert.equal(compactDiscussionSummary('![](img.png)'), 'Untitled discussion')
  assert.equal(
    compactDiscussionSummary('![screenshot](img.png)'),
    'Untitled discussion',
  )
})

test('blockquote prefixes are stripped', () => {
  assert.equal(compactDiscussionSummary('> quoted line'), 'quoted line')
  assert.equal(compactDiscussionSummary('> > deeply quoted'), 'deeply quoted')
  assert.equal(compactDiscussionSummary('>> ## quoted heading'), 'quoted heading')
})

test('list markers are stripped', () => {
  assert.equal(compactDiscussionSummary('- first item'), 'first item')
  assert.equal(compactDiscussionSummary('* first item'), 'first item')
  assert.equal(compactDiscussionSummary('+ first item'), 'first item')
  assert.equal(compactDiscussionSummary('1. first item'), 'first item')
  assert.equal(compactDiscussionSummary('  12. indented item'), 'indented item')
})

test('whitespace runs collapse to single spaces', () => {
  assert.equal(
    compactDiscussionSummary('too    many\t\tgaps  here  '),
    'too many gaps here',
  )
})

test('combined markdown renders as plain prose', () => {
  assert.equal(
    compactDiscussionSummary(
      '> - **rename** `retryDelay` per [the docs](https://x.dev) ![](i.png)',
    ),
    'rename retryDelay per the docs',
  )
})
