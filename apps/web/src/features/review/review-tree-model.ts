import type { ReviewFile, VisibilityState } from '@/api/types'

export type ReviewTreeNode<TFile extends ReviewFile = ReviewFile> =
  | {
      children: ReviewTreeNode<TFile>[]
      files: TFile[]
      key: string
      name: string
      path: string
      type: 'folder'
    }
  | {
      file: TFile
      key: string
      name: string
      path: string
      type: 'file'
    }

export function buildReviewTree<TFile extends ReviewFile>(files: TFile[]) {
  const root: Extract<ReviewTreeNode<TFile>, { type: 'folder' }> = {
    children: [],
    files: [],
    key: 'folder:/',
    name: '',
    path: '/',
    type: 'folder',
  }

  for (const file of files) {
    const path = normalizeReviewPath(file.path)
    const parts = pathParts(path)
    if (parts.length === 0) {
      continue
    }

    let current = root
    for (let index = 0; index < parts.length; index += 1) {
      const part = parts[index]
      const folderPath = `/${parts.slice(0, index + 1).join('/')}`
      const last = index === parts.length - 1
      if (last) {
        current.children.push({
          file,
          key: `file:${path}`,
          name: part,
          path,
          type: 'file',
        })
      } else {
        let folder = current.children.find(
          (child): child is Extract<ReviewTreeNode<TFile>, { type: 'folder' }> =>
            child.type === 'folder' && child.path === folderPath,
        )
        if (!folder) {
          folder = {
            children: [],
            files: [],
            key: `folder:${folderPath}`,
            name: part,
            path: folderPath,
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
  return normalizeReviewPath(path)
}

export function normalizeReviewPath(path: string) {
  return path
    .replace(/\\/g, '/')
    .split('/')
    .filter((part) => part && part !== '.' && part !== '..')
    .join('/')
}

function sortReviewTree<TFile extends ReviewFile>(
  node: Extract<ReviewTreeNode<TFile>, { type: 'folder' }>,
) {
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

function attachDescendantFiles<TFile extends ReviewFile>(
  node: Extract<ReviewTreeNode<TFile>, { type: 'folder' }>,
) {
  node.files = node.children.flatMap((child) => {
    if (child.type === 'file') {
      return [child.file]
    }
    attachDescendantFiles(child)
    return child.files
  })
}

function pathParts(path: string) {
  return path.split('/').filter(Boolean)
}
