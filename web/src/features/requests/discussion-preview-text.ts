const FENCE = /^(?:`{3,}|~{3,})/
const BLOCKQUOTE = /^>\s?/
const LIST_MARKER = /^(?:[-*+]|\d+\.)\s+/
const HEADING = /^#{1,6}\s+/
const IMAGE = /!\[[^\]]*\]\([^)]*\)/g
const LINK = /\[([^\]]*)\]\([^)]*\)/g
const BACKTICKS = /`+/g
const STRIKETHROUGH = /~~(\S(?:[\s\S]*?\S)?)~~/g
const BOLD_STAR = /\*\*(\S(?:[\s\S]*?\S)?)\*\*/g
const ITALIC_STAR = /\*(\S(?:[\s\S]*?\S)?)\*/g
const BOLD_UNDERSCORE = /(?<![\w])__(\S(?:[\s\S]*?\S)?)__(?![\w])/g
const ITALIC_UNDERSCORE = /(?<![\w])_(\S(?:[\s\S]*?\S)?)_(?![\w])/g
const WHITESPACE = /\s+/g

/**
 * First meaningful line of a discussion body, rendered as plain text so
 * collapsed threads and quote chips never show raw markdown syntax.
 */
export function compactDiscussionSummary(body: string | null) {
  if (!body) return 'Update'
  for (const line of body.split('\n')) {
    const text = plainTextLine(line)
    if (text) return text
  }
  return 'Untitled discussion'
}

function plainTextLine(line: string) {
  let text = line.trim()
  if (FENCE.test(text)) return ''
  while (BLOCKQUOTE.test(text)) {
    text = text.replace(BLOCKQUOTE, '').trim()
  }
  return text
    .replace(LIST_MARKER, '')
    .replace(HEADING, '')
    .replace(IMAGE, '')
    .replace(LINK, '$1')
    .replace(BACKTICKS, '')
    .replace(STRIKETHROUGH, '$1')
    .replace(BOLD_STAR, '$1')
    .replace(BOLD_UNDERSCORE, '$1')
    .replace(ITALIC_STAR, '$1')
    .replace(ITALIC_UNDERSCORE, '$1')
    .replace(WHITESPACE, ' ')
    .trim()
}
