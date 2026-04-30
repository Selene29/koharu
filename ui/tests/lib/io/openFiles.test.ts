import { describe, expect, it, vi } from 'vitest'

async function importOpenFilesWithTauri(readDir: (path: string) => Promise<unknown[]>) {
  vi.resetModules()
  vi.doMock('@/lib/backend', () => ({ isTauri: () => true }))
  vi.doMock('@tauri-apps/plugin-dialog', () => ({
    open: vi.fn().mockResolvedValue('C:/Users/Pascal/Downloads/Manga'),
  }))
  vi.doMock('@tauri-apps/plugin-fs', () => ({
    readDir,
  }))
  return import('@/lib/io/openFiles')
}

async function importOpenFilesWithWeb(files: File[]) {
  vi.resetModules()
  vi.doMock('@/lib/backend', () => ({ isTauri: () => false }))
  vi.doMock('browser-fs-access', () => ({
    directoryOpen: vi.fn().mockResolvedValue(files),
  }))
  return import('@/lib/io/openFiles')
}

describe('openImageFolder', () => {
  it('recursively returns Tauri image paths with relative paths', async () => {
    const readDir = vi.fn(async (path: string) => {
      const tree: Record<string, unknown[]> = {
        'C:/Users/Pascal/Downloads/Manga': [
          { name: 'Chapter 2', isFile: false, isDirectory: true, isSymlink: false },
          { name: 'Chapter 1', isFile: false, isDirectory: true, isSymlink: false },
          { name: 'notes.txt', isFile: true, isDirectory: false, isSymlink: false },
          { name: 'linked', isFile: false, isDirectory: true, isSymlink: true },
        ],
        'C:/Users/Pascal/Downloads/Manga/Chapter 1': [
          { name: '010.png', isFile: true, isDirectory: false, isSymlink: false },
          { name: '002.jpg', isFile: true, isDirectory: false, isSymlink: false },
        ],
        'C:/Users/Pascal/Downloads/Manga/Chapter 2': [
          { name: '001.webp', isFile: true, isDirectory: false, isSymlink: false },
        ],
      }
      return tree[path] ?? []
    })
    const { openImageFolder } = await importOpenFilesWithTauri(readDir)

    await expect(openImageFolder()).resolves.toEqual({
      kind: 'paths',
      entries: [
        {
          path: 'C:/Users/Pascal/Downloads/Manga/Chapter 1/002.jpg',
          relativePath: 'Chapter 1/002.jpg',
        },
        {
          path: 'C:/Users/Pascal/Downloads/Manga/Chapter 1/010.png',
          relativePath: 'Chapter 1/010.png',
        },
        {
          path: 'C:/Users/Pascal/Downloads/Manga/Chapter 2/001.webp',
          relativePath: 'Chapter 2/001.webp',
        },
      ],
    })
  })

  it('uses webkitRelativePath for recursive web folder imports', async () => {
    const ch2 = new File([new Uint8Array([2])], '001.png', { type: 'image/png' })
    Object.defineProperty(ch2, 'webkitRelativePath', {
      value: 'Manga/Chapter 2/001.png',
    })
    const ch1 = new File([new Uint8Array([1])], '010.png', { type: 'image/png' })
    Object.defineProperty(ch1, 'webkitRelativePath', {
      value: 'Manga/Chapter 1/010.png',
    })
    const ignored = new File([new Uint8Array([0])], 'notes.txt', { type: 'text/plain' })
    Object.defineProperty(ignored, 'webkitRelativePath', {
      value: 'Manga/Chapter 1/notes.txt',
    })
    const { openImageFolder } = await importOpenFilesWithWeb([ch2, ignored, ch1])

    await expect(openImageFolder()).resolves.toEqual({
      kind: 'files',
      entries: [
        { file: ch1, relativePath: 'Chapter 1/010.png' },
        { file: ch2, relativePath: 'Chapter 2/001.png' },
      ],
    })
  })
})
