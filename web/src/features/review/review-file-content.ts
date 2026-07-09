import type { ReviewFileDiff } from '@/api/types'

export type ReviewFileContent = NonNullable<ReviewFileDiff['old_content']>

export type BinaryContentSide = {
  label: string
  oid: string
  sizeBytes: number
}

export type TextContentSide = {
  label: string
  text: string
}

export function reviewContentSides(diff: ReviewFileDiff) {
  const binary: BinaryContentSide[] = []
  const text: TextContentSide[] = []

  addSide('Old', diff.old_content, binary, text)
  addSide('New', diff.new_content, binary, text)

  return { binary, text }
}

function addSide(
  label: string,
  content: ReviewFileContent | null,
  binary: BinaryContentSide[],
  text: TextContentSide[],
) {
  if (!content) return
  if (content.kind === 'binary') {
    binary.push({ label, oid: content.oid, sizeBytes: content.size_bytes })
  } else {
    text.push({ label, text: content.text })
  }
}
