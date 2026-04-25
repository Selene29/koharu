'use client'

import { getGetSceneJsonQueryKey } from '@/lib/api/default/default'
import type { SceneSnapshot } from '@/lib/api/schemas'
import { openImageFiles, openImageFolder, openKhrFile } from '@/lib/io/openFiles'
import { saveBlob } from '@/lib/io/saveBlob'
import { exportProject, uploadKhrArchive, uploadPages, uploadPagesByPaths } from '@/lib/io/scene'
import { queryClient } from '@/lib/queryClient'
import { useActivityLogStore } from '@/lib/stores/activityLogStore'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'

/**
 * Platform-neutral image import. `openImageFiles` / `openImageFolder` return
 * `File[]` on both Tauri and the web; the upload + scene invalidation lives
 * in `lib/io/scene.ts` on top of the orval-generated `createPages` mutation.
 */
export async function importPages(
  mode: 'replace' | 'append',
  source: 'files' | 'folder',
): Promise<void> {
  const picked = source === 'folder' ? await openImageFolder() : await openImageFiles()
  const replace = mode === 'replace'
  if (picked.kind === 'paths') {
    if (picked.paths.length === 0) return
    await uploadPagesByPaths(picked.paths, replace)
    return
  }
  if (picked.files.length === 0) return
  await uploadPages(picked.files, replace)
}

/**
 * Import a `.khr` archive. Works on both desktop and web: the archive file
 * is picked via the cross-platform `openKhrFile`, and the destination is
 * allocated server-side so the client never needs to touch the filesystem.
 */
export async function importKhrFile(): Promise<void> {
  const file = await openKhrFile()
  if (!file) return
  await uploadKhrArchive(file)
}

// ---------------------------------------------------------------------------
// Export (server returns bytes; saveBlob dispatches Tauri-dialog / web-FS)
// ---------------------------------------------------------------------------

const exportExtension: Record<'khr' | 'psd' | 'rendered' | 'inpainted', string> = {
  khr: 'khr',
  psd: 'zip',
  rendered: 'zip',
  inpainted: 'zip',
}

/** Sanitise an arbitrary project name for use as a filename stem. */
function sanitiseBaseName(name: string | undefined | null): string {
  const cleaned = (name ?? '')
    .trim()
    .replace(/[\\/:*?"<>|]+/g, '_')
    .replace(/\s+/g, ' ')
  return cleaned.length > 0 ? cleaned : 'koharu-export'
}

/** Read the current project name from React Query's cached scene snapshot. */
function currentProjectName(): string | undefined {
  const snap = queryClient.getQueryData<SceneSnapshot>(getGetSceneJsonQueryKey())
  return snap?.scene.project?.name ?? undefined
}

export async function exportCurrentProjectAs(
  format: 'khr' | 'psd' | 'rendered' | 'inpainted',
  pages?: string[],
): Promise<void> {
  const log = useActivityLogStore.getState()
  const scope = pages?.length === 1 ? 'current page' : pages ? `${pages.length} pages` : 'all pages'
  log.push('info', `Export ${format} (${scope}) — requesting from server`)

  try {
    const { blob, filename } = await exportProject({ format, pages })
    log.push(
      'info',
      `Export ${format}: server returned ${(blob.size / 1024).toFixed(1)} KB`,
      `type=${blob.type || 'unknown'}; filename=${filename ?? '(none)'}`,
    )

    if (blob.size === 0) {
      const msg = `Export ${format}: server returned empty body`
      log.push('error', msg)
      useEditorUiStore.getState().showError(msg)
      return
    }

    const base = sanitiseBaseName(currentProjectName())
    // Prefer the server's Content-Disposition filename (matches the actual
    // bytes — a raw PNG/PSD for single-file responses, a zip for multi).
    // Fall back to our guess only if the header is missing/unparseable.
    const defaultName = filename ?? `${base}.${exportExtension[format]}`

    log.push('info', `Export ${format}: opening save dialog (${defaultName})`)
    const saved = await saveBlob(blob, defaultName)
    if (saved) {
      log.push('info', `Export ${format}: saved as ${defaultName}`)
    } else {
      log.push('info', `Export ${format}: cancelled by user`)
    }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    console.error('Export failed:', err)
    log.push('error', `Export ${format} failed`, msg)
    useEditorUiStore.getState().showError(`Export ${format} failed: ${msg}`)
    throw err
  }
}
