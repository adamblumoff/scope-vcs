import assert from 'node:assert/strict'
import test from 'node:test'
import { compactDiscussionSummary } from './discussion-preview-text'

const cases: [body: string | null, expected: string][] = [
  [null, 'Update'],
  ['', 'Update'],
  [' \n\t\n   ', 'Untitled discussion'],
  ['\n## Cache invalidation\nMore', 'Cache invalidation'],
  ['###### Deep', 'Deep'],
  ['the name `retryDelay` does not state the unit', 'the name retryDelay does not state the unit'],
  ['use ``a ` b`` here', 'use a b here'],
  ['```ts\nconst retryDelay = 500\n```', 'const retryDelay = 500'],
  ['~~~\nplain fence body\n~~~', 'plain fence body'],
  ['**bold** start', 'bold start'],
  ['__bold__ start', 'bold start'],
  ['*italic* start', 'italic start'],
  ['_italic_ start', 'italic start'],
  ['~~gone~~ now', 'gone now'],
  ['**bold _nested_ text**', 'bold nested text'],
  ['rename retry_delay_ms please', 'rename retry_delay_ms please'],
  ['see [the docs](https://example.com/a_b) first', 'see the docs first'],
  ['before ![alt text](img.png) after', 'before after'],
  ['before ![](img.png) after', 'before after'],
  ['![](img.png)', 'Untitled discussion'],
  ['![screenshot](img.png)', 'Untitled discussion'],
  ['> quoted line', 'quoted line'],
  ['> > deeply quoted', 'deeply quoted'],
  ['>> ## quoted heading', 'quoted heading'],
  ['- first item', 'first item'],
  ['* first item', 'first item'],
  ['+ first item', 'first item'],
  ['1. first item', 'first item'],
  ['  12. indented item', 'indented item'],
  ['too    many\t\tgaps  here  ', 'too many gaps here'],
  [
    '> - **rename** `retryDelay` per [the docs](https://x.dev) ![](i.png)',
    'rename retryDelay per the docs',
  ],
]

test('discussion previews reduce markdown to compact plain text', () => {
  for (const [body, expected] of cases) {
    assert.equal(compactDiscussionSummary(body), expected, body ?? 'null')
  }
})
