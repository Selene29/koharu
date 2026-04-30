'use client'

/**
 * Unified open-file pickers that use the Tauri dialog plugin when available
 * and fall back to the web File System Access API (via `browser-fs-access`)
 * otherwise. Folder imports also carry a relative path so exports can rebuild
 * chapter subdirectories.
 *
 * Directory picking is supported on web via `directoryOpen`, which uses the
 * File System Access API in Chromium and falls back to `<input webkitdirectory>`
 * on Firefox/Safari.
 */

import { isTauri } from '@/lib/backend'

const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'webp'] as const
const IMAGE_MIME = ['image/png', 'image/jpeg', 'image/webp']
const IMAGE_RE = /\.(png|jpe?g|webp)$/i

/**
 * Platform-tagged picker result. On Tauri we hand paths straight to the
 * backend (no JS-side file read — backend reads from disk in parallel);
 * on the web we must round-trip through `File` since we can't escape the
 * sandbox.
 */
export type ImagePickerResult =
  | { kind: 'paths'; entries: ImagePathEntry[] }
  | { kind: 'files'; entries: ImageFileEntry[] }

export type ImagePathEntry = {
  path: string
  relativePath?: string
}

export type ImageFileEntry = {
  file: File
  relativePath?: string
}

/** Pick one or more image files. Empty result = user cancelled. */
export async function openImageFiles(): Promise<ImagePickerResult> {
  if (isTauri()) {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const picked = await open({
      multiple: true,
      filters: [{ name: 'Images', extensions: [...IMAGE_EXTENSIONS] }],
    })
    if (!picked) return { kind: 'paths', entries: [] }
    const paths = Array.isArray(picked) ? picked : [picked]
    return { kind: 'paths', entries: paths.map((path) => ({ path })) }
  }

  const { fileOpen } = await import('browser-fs-access')
  try {
    const result = await fileOpen({
      multiple: true,
      mimeTypes: IMAGE_MIME,
      extensions: IMAGE_EXTENSIONS.map((e) => `.${e}`),
      description: 'Images',
    })
    const files = Array.isArray(result) ? result : [result]
    return { kind: 'files', entries: files.map((file) => ({ file })) }
  } catch (e) {
    if (isAbort(e)) return { kind: 'files', entries: [] }
    throw e
  }
}

/** Pick a folder; return every image file inside it recursively. */
export async function openImageFolder(): Promise<ImagePickerResult> {
  if (isTauri()) {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const folder = await open({ directory: true, multiple: false })
    if (!folder || typeof folder !== 'string') return { kind: 'paths', entries: [] }
    const { readDir } = await import('@tauri-apps/plugin-fs')
    const entries = await collectTauriImageEntries(folder, folder, readDir)
    entries.sort(comparePickerEntries)
    return { kind: 'paths', entries }
  }

  const { directoryOpen } = await import('browser-fs-access')
  try {
    const results = await directoryOpen({ recursive: true })
    const arr = Array.isArray(results) ? results : [results]
    const entries = arr
      .filter((f): f is File => !!f && IMAGE_RE.test(f.name))
      .map((file) => ({
        file,
        relativePath: stripPickedRoot(
          (file as File & { webkitRelativePath?: string }).webkitRelativePath,
        ),
      }))
    entries.sort(comparePickerEntries)
    return { kind: 'files', entries }
  } catch (e) {
    if (isAbort(e)) return { kind: 'files', entries: [] }
    throw e
  }
}

/** Pick one `.khr` archive file. Returns `null` if cancelled. */
export async function openKhrFile(): Promise<File | null> {
  if (isTauri()) {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const picked = await open({
      multiple: false,
      filters: [{ name: 'Koharu archive', extensions: ['khr'] }],
    })
    if (!picked || typeof picked !== 'string') return null
    const [file] = await readTauriFiles([picked])
    return file ?? null
  }

  const { fileOpen } = await import('browser-fs-access')
  try {
    const result = await fileOpen({
      multiple: false,
      extensions: ['.khr'],
      description: 'Koharu archive',
    })
    return Array.isArray(result) ? (result[0] ?? null) : result
  } catch (e) {
    if (isAbort(e)) return null
    throw e
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function readTauriFiles(paths: string[]): Promise<File[]> {
  if (paths.length === 0) return []
  const { readFile } = await import('@tauri-apps/plugin-fs')
  const out: File[] = []
  for (const path of paths) {
    const bytes = await readFile(path)
    const name = path.split(/[\\/]/).pop() || 'file'
    out.push(new File([bytes as unknown as BlobPart], name, { type: mimeFromName(name) }))
  }
  return out
}

async function collectTauriImageEntries(
  root: string,
  dir: string,
  readDir: (
    path: string,
  ) => Promise<Array<{ name: string; isFile: boolean; isDirectory: boolean; isSymlink: boolean }>>,
): Promise<ImagePathEntry[]> {
  const entries = await readDir(dir)
  const out: ImagePathEntry[] = []
  for (const entry of entries) {
    if (!entry.name || entry.isSymlink) continue
    const fullPath = joinPath(dir, entry.name)
    if (entry.isDirectory) {
      out.push(...(await collectTauriImageEntries(root, fullPath, readDir)))
      continue
    }
    if (!entry.isFile || !IMAGE_RE.test(entry.name)) continue
    out.push({
      path: fullPath,
      relativePath: relativeFromRoot(root, fullPath),
    })
  }
  return out
}

function joinPath(dir: string, name: string): string {
  return `${dir.replace(/[\\/]+$/g, '')}/${name}`
}

function relativeFromRoot(root: string, path: string): string | undefined {
  const normalizedRoot = root.replace(/\\/g, '/').replace(/\/+$/g, '')
  const normalizedPath = path.replace(/\\/g, '/')
  const prefix = `${normalizedRoot}/`
  if (!normalizedPath.startsWith(prefix)) return undefined
  return normalizedPath.slice(prefix.length)
}

function stripPickedRoot(path: string | undefined): string | undefined {
  if (!path) return undefined
  const normalized = path.replace(/\\/g, '/')
  const slash = normalized.indexOf('/')
  return slash >= 0 ? normalized.slice(slash + 1) : normalized
}

const naturalPathCollator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: 'base',
})

function comparePickerEntries(
  a: ImagePathEntry | ImageFileEntry,
  b: ImagePathEntry | ImageFileEntry,
): number {
  const aPath = a.relativePath ?? ('path' in a ? a.path : a.file.name)
  const bPath = b.relativePath ?? ('path' in b ? b.path : b.file.name)
  return naturalPathCollator.compare(aPath, bPath)
}

function mimeFromName(name: string): string {
  const lower = name.toLowerCase()
  if (lower.endsWith('.png')) return 'image/png'
  if (lower.endsWith('.jpg') || lower.endsWith('.jpeg')) return 'image/jpeg'
  if (lower.endsWith('.webp')) return 'image/webp'
  if (lower.endsWith('.khr')) return 'application/zip'
  return 'application/octet-stream'
}

function isAbort(e: unknown): boolean {
  if (typeof e !== 'object' || e === null) return false
  const err = e as { name?: string }
  return err.name === 'AbortError'
}
