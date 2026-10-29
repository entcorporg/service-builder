// Typed access to the Rust side (Tauri commands) and to native dialogs.
import { invoke } from '@tauri-apps/api/core'
import { getVersion } from '@tauri-apps/api/app'
import { ask, open, save } from '@tauri-apps/plugin-dialog'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'
import type { Analysis, ExportReport, McpActivation, McpInfo, Project } from './types'

export const api = {
  sampleProject: () => invoke<Project>('sample_project'),
  newProject: (name: string) => invoke<Project>('new_project', { name }),
  analyze: (project: Project) => invoke<Analysis>('analyze', { project }),
  openProject: (path: string) => invoke<Project>('open_project', { path }),
  saveProject: (path: string, project: Project) => invoke<void>('save_project', { path, project }),
  exportProject: (project: Project, dir: string) => invoke<ExportReport>('export_project', { project, dir }),
  mcpInfo: (path: string | null) => invoke<McpInfo>('mcp_info', { path }),
  mcpSetEnabled: (path: string, enabled: boolean) => invoke<McpActivation>('mcp_set_enabled', { path, enabled }),
  projectStamp: (path: string) => invoke<number>('project_stamp', { path }),
}

const PROJECT_FILTER = [{ name: 'Service Builder', extensions: ['sb.json', 'json'] }]

export async function pickProjectToOpen(): Promise<string | null> {
  const picked = await open({ multiple: false, directory: false, filters: PROJECT_FILTER })
  return typeof picked === 'string' ? picked : null
}

export async function pickProjectToSave(defaultName: string): Promise<string | null> {
  return save({ defaultPath: `${defaultName}.sb.json`, filters: PROJECT_FILTER })
}

export async function pickExportDir(): Promise<string | null> {
  const picked = await open({ multiple: false, directory: true, title: 'Dossier de destination du code généré' })
  return typeof picked === 'string' ? picked : null
}

export function confirm(message: string): Promise<boolean> {
  return ask(message, { title: 'Service Builder', kind: 'warning', okLabel: 'Continuer', cancelLabel: 'Annuler' })
}

export interface AvailableUpdate {
  version: string
  notes: string | null
}

// The handle returned by `check` is the only way to install what it found.
let found: Update | null = null

export const updater = {
  currentVersion: () => getVersion(),

  /** Asks the GitHub release feed whether a newer signed build exists. */
  async check(): Promise<AvailableUpdate | null> {
    await found?.close().catch(() => {})
    found = await check()
    return found ? { version: found.version, notes: found.body ?? null } : null
  },

  /** Downloads, verifies the signature, installs, then restarts. `percent` is null while the size is unknown. */
  async install(onProgress: (percent: number | null) => void): Promise<void> {
    if (!found) throw new Error('aucune mise à jour trouvée')
    let total = 0
    let done = 0
    await found.downloadAndInstall((event) => {
      if (event.event === 'Started') total = event.data.contentLength ?? 0
      else if (event.event === 'Progress') {
        done += event.data.chunkLength
        onProgress(total > 0 ? Math.min(100, Math.round((done / total) * 100)) : null)
      }
    })
    await relaunch()
  },
}
