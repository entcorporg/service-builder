// User-level operations: they talk to the backend and update the store.
import { api, confirm, pickExportDir, pickProjectToOpen, pickProjectToSave } from './bridge'
import { useStore } from './store'
import type { Project } from './types'

const store = () => useStore.getState()

function fail(e: unknown) {
  store().notify({ kind: 'error', text: String(e) })
}

async function confirmDiscard(): Promise<boolean> {
  if (!store().dirty) return true
  return confirm('Le projet a des modifications non enregistrées. Les abandonner ?')
}

export async function newProject() {
  if (!(await confirmDiscard())) return
  try {
    store().load(await api.newProject('Nouveau projet'), null, true)
  } catch (e) {
    fail(e)
  }
}

export async function loadSample() {
  if (!(await confirmDiscard())) return
  try {
    store().load(await api.sampleProject(), null, true)
  } catch (e) {
    fail(e)
  }
}

export async function openProject() {
  if (!(await confirmDiscard())) return
  try {
    const path = await pickProjectToOpen()
    if (!path) return
    store().load(await api.openProject(path), path)
    store().notify({ kind: 'info', text: `Projet ouvert : ${path}` })
  } catch (e) {
    fail(e)
  }
}

async function writeTo(path: string, project: Project) {
  await api.saveProject(path, project)
  store().markSaved(path)
  store().notify({ kind: 'info', text: `Enregistré : ${path}` })
}

export async function saveProject() {
  const { filePath, project } = store()
  try {
    const path = filePath ?? (await pickProjectToSave(project.name.replace(/\s+/g, '-').toLowerCase()))
    if (path) await writeTo(path, project)
  } catch (e) {
    fail(e)
  }
}

export async function saveProjectAs() {
  const { project } = store()
  try {
    const path = await pickProjectToSave(project.name.replace(/\s+/g, '-').toLowerCase())
    if (path) await writeTo(path, project)
  } catch (e) {
    fail(e)
  }
}

export async function exportCode() {
  const { project } = store()
  try {
    const dir = await pickExportDir()
    if (!dir) return
    const report = await api.exportProject(project, dir)
    store().notify({ kind: 'info', text: `${report.files} fichiers générés dans ${report.dir}` })
  } catch (e) {
    fail(e)
  }
}
