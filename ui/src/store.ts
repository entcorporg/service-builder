import { create } from 'zustand'
import { immer } from 'zustand/middleware/immer'
import type { Selection } from './model'
import type { Analysis, GeneratedFile, Project } from './types'

export type View = 'architecture' | 'tables' | 'code' | 'mcp'

export interface Notice {
  kind: 'info' | 'error'
  text: string
}

export type UpdateStatus =
  | { phase: 'idle' }
  | { phase: 'checking' }
  | { phase: 'available'; version: string; notes: string | null }
  | { phase: 'installing'; percent: number | null }

interface AppState {
  project: Project
  filePath: string | null
  dirty: boolean
  selection: Selection
  view: View
  analysis: Analysis
  /** Last successfully generated files (kept while the project has errors). */
  files: GeneratedFile[]
  filesStale: boolean
  activeFile: string | null
  problemsOpen: boolean
  notice: Notice | null
  update: UpdateStatus

  edit: (fn: (draft: Project) => void) => void
  load: (project: Project, filePath: string | null, dirty?: boolean) => void
  select: (selection: Selection) => void
  setView: (view: View) => void
  setAnalysis: (analysis: Analysis) => void
  setActiveFile: (path: string) => void
  toggleProblems: () => void
  markSaved: (filePath: string) => void
  notify: (notice: Notice | null) => void
  setUpdate: (update: UpdateStatus) => void
}

const emptyProject: Project = {
  schemaVersion: 3,
  name: 'Nouveau projet',
  broker: { bootstrapServers: 'localhost:9092' },
  outbox: { maxAttempts: 5 },
  messages: [],
  topics: [],
  services: [],
}

export const useStore = create<AppState>()(
  immer((set) => ({
    project: emptyProject,
    filePath: null,
    dirty: false,
    selection: null,
    view: 'architecture',
    analysis: { diagnostics: [], files: [] },
    files: [],
    filesStale: false,
    activeFile: null,
    problemsOpen: true,
    notice: null,
    update: { phase: 'idle' },

    edit: (fn) =>
      set((s) => {
        fn(s.project)
        s.dirty = true
      }),
    load: (project, filePath, dirty = false) =>
      set((s) => {
        s.project = project
        s.filePath = filePath
        s.dirty = dirty
        s.selection = null
        s.files = []
        s.activeFile = null
      }),
    select: (selection) =>
      set((s) => {
        s.selection = selection
      }),
    setView: (view) =>
      set((s) => {
        s.view = view
      }),
    setAnalysis: (analysis) =>
      set((s) => {
        s.analysis = { diagnostics: analysis.diagnostics, files: [] }
        if (analysis.files.length > 0) {
          s.files = analysis.files
          s.filesStale = false
          if (!s.activeFile || !analysis.files.some((f) => f.path === s.activeFile)) {
            s.activeFile = analysis.files.find((f) => /\/flows\/(?!mod\.rs)/.test(f.path))?.path ?? analysis.files[0]!.path
          }
        } else {
          s.filesStale = true
        }
      }),
    setActiveFile: (path) =>
      set((s) => {
        s.activeFile = path
      }),
    toggleProblems: () =>
      set((s) => {
        s.problemsOpen = !s.problemsOpen
      }),
    markSaved: (filePath) =>
      set((s) => {
        s.filePath = filePath
        s.dirty = false
      }),
    notify: (notice) =>
      set((s) => {
        s.notice = notice
      }),
    setUpdate: (update) =>
      set((s) => {
        s.update = update
      }),
  })),
)

export const errorCount = (a: Analysis) => a.diagnostics.filter((d) => d.severity === 'error').length
export const warningCount = (a: Analysis) => a.diagnostics.filter((d) => d.severity === 'warning').length
