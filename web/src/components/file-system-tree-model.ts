import type { Visibility, VisibilityState } from '@/api/types'

export type FileSystemTreeFileBase = {
  path: string
  visibility: Visibility
}

export type FileSystemTreeNode<
  TFile extends FileSystemTreeFileBase = FileSystemTreeFileBase,
> =
  | {
      children: FileSystemTreeNode<TFile>[]
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

export function buildFileSystemTree<TFile extends FileSystemTreeFileBase>(
  files: TFile[],
) {
  const root: Extract<FileSystemTreeNode<TFile>, { type: 'folder' }> = {
    children: [],
    files: [],
    key: 'folder:/',
    name: '',
    path: '/',
    type: 'folder',
  }
  const foldersByPath = new Map<string, typeof root>([[root.path, root]])

  for (const file of files) {
    const path = normalizeFilePath(file.path)
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
        let folder = foldersByPath.get(folderPath)
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
          foldersByPath.set(folderPath, folder)
        }
        current = folder
      }
    }
  }

  sortFileSystemTree(root)
  attachDescendantFiles(root)
  return root
}

export function folderVisibility(
  files: FileSystemTreeFileBase[],
): VisibilityState {
  const hasPublic = files.some((file) => file.visibility === 'Public')
  const hasPrivate = files.some((file) => file.visibility === 'Private')
  if (hasPublic && hasPrivate) {
    return 'Mixed'
  }
  return hasPublic ? 'Public' : 'Private'
}

export function folderCollapseKeys<TFile extends FileSystemTreeFileBase>(
  node: Extract<FileSystemTreeNode<TFile>, { type: 'folder' }>,
): string[] {
  return node.children.flatMap((child) => {
    if (child.type === 'file') {
      return []
    }
    return [child.key, ...folderCollapseKeys(child)]
  })
}

export function ancestorFolderKeys(path: string) {
  const parts = pathParts(normalizeFilePath(path)).slice(0, -1)
  return parts.map(
    (_, index) => `folder:/${parts.slice(0, index + 1).join('/')}`,
  )
}

export function displayPath(path: string) {
  return normalizeFilePath(path)
}

export function normalizeFilePath(path: string) {
  return path
    .replace(/\\/g, '/')
    .split('/')
    .filter((part) => part && part !== '.' && part !== '..')
    .join('/')
}

function sortFileSystemTree<TFile extends FileSystemTreeFileBase>(
  node: Extract<FileSystemTreeNode<TFile>, { type: 'folder' }>,
) {
  node.children.sort((left, right) => {
    if (left.type !== right.type) {
      return left.type === 'folder' ? -1 : 1
    }
    return left.name.localeCompare(right.name)
  })
  for (const child of node.children) {
    if (child.type === 'folder') {
      sortFileSystemTree(child)
    }
  }
}

function attachDescendantFiles<TFile extends FileSystemTreeFileBase>(
  node: Extract<FileSystemTreeNode<TFile>, { type: 'folder' }>,
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
