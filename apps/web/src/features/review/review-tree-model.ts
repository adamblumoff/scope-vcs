import type { ReviewFile, VisibilityState } from '@/api/types'

export type ReviewTreeNode =
  | {
      children: ReviewTreeNode[]
      files: ReviewFile[]
      key: string
      name: string
      path: string
      type: 'folder'
    }
  | {
      file: ReviewFile
      key: string
      name: string
      path: string
      type: 'file'
    }

export function buildReviewTree(files: ReviewFile[]) {
  const root: Extract<ReviewTreeNode, { type: 'folder' }> = {
    children: [],
    files: [],
    key: 'folder:/',
    name: '',
    path: '/',
    type: 'folder',
  }

  for (const file of files) {
    const parts = pathParts(file.path)
    let current = root
    for (let index = 0; index < parts.length; index += 1) {
      const part = parts[index]
      const path = `/${parts.slice(0, index + 1).join('/')}`
      const last = index === parts.length - 1
      if (last) {
        current.children.push({
          file,
          key: `file:${file.path}`,
          name: part,
          path: file.path,
          type: 'file',
        })
      } else {
        let folder = current.children.find(
          (child): child is Extract<ReviewTreeNode, { type: 'folder' }> =>
            child.type === 'folder' && child.path === path,
        )
        if (!folder) {
          folder = {
            children: [],
            files: [],
            key: `folder:${path}`,
            name: part,
            path,
            type: 'folder',
          }
          current.children.push(folder)
        }
        current = folder
      }
    }
  }

  sortReviewTree(root)
  attachDescendantFiles(root)
  return root
}

export function folderVisibility(files: ReviewFile[]): VisibilityState {
  const hasPublic = files.some((file) => file.visibility === 'Public')
  const hasPrivate = files.some((file) => file.visibility === 'Private')
  if (hasPublic && hasPrivate) {
    return 'Mixed'
  }
  return hasPublic ? 'Public' : 'Private'
}

export function displayPath(path: string) {
  return path.replace(/^\/+/, '')
}

function sortReviewTree(node: Extract<ReviewTreeNode, { type: 'folder' }>) {
  node.children.sort((left, right) => {
    if (left.type !== right.type) {
      return left.type === 'folder' ? -1 : 1
    }
    return left.name.localeCompare(right.name)
  })
  for (const child of node.children) {
    if (child.type === 'folder') {
      sortReviewTree(child)
    }
  }
}

function attachDescendantFiles(node: Extract<ReviewTreeNode, { type: 'folder' }>) {
  node.files = node.children.flatMap((child) => {
    if (child.type === 'file') {
      return [child.file]
    }
    attachDescendantFiles(child)
    return child.files
  })
}

function pathParts(path: string) {
  return path.replace(/^\/+/, '').split('/').filter(Boolean)
}
