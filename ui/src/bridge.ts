// Typed access to the Rust side (Tauri commands) and to native dialogs.
import { invoke } from '@tauri-apps/api/core'
import { ask, open, save } from '@tauri-apps/plugin-dialog'
import type { Analysis, ExportReport, Project } from './types'

export const api = {
  sampleProject: () => invoke<Project>('sample_project'),
  newProject: (name: string) => invoke<Project>('new_project', { name }),
  analyze: (project: Project) => invoke<Analysis>('analyze', { project }),
  openProject: (path: string) => invoke<Project>('open_project', { path }),
  saveProject: (path: string, project: Project) => invoke<void>('save_project', { path, project }),
  exportProject: (project: Project, dir: string) => invoke<ExportReport>('export_project', { project, dir }),
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
