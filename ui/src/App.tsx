import { useEffect, useRef } from 'react'
import { checkForUpdate, openProject, saveProject, saveProjectAs } from './actions'
import { api } from './bridge'
import { Canvas } from './components/Canvas'
import { EndpointEditor } from './components/EndpointEditor'
import { CodeView } from './components/CodeView'
import { Inspector } from './components/Inspector'
import { Palette } from './components/Palette'
import { Problems } from './components/Problems'
import { TablesView } from './components/TablesView'
import { Toolbar } from './components/Toolbar'
import { useStore } from './store'

/** Re-validates and regenerates after every edit, ignoring answers that arrive out of order. */
function useAnalysis() {
  const project = useStore((s) => s.project)
  const setAnalysis = useStore((s) => s.setAnalysis)
  const notify = useStore((s) => s.notify)
  const seq = useRef(0)
  useEffect(() => {
    const mine = ++seq.current
    const timer = setTimeout(() => {
      api
        .analyze(project)
        .then((a) => mine === seq.current && setAnalysis(a))
        .catch((e) => notify({ kind: 'error', text: `Analyse impossible : ${String(e)}` }))
    }, 250)
    return () => clearTimeout(timer)
  }, [project, setAnalysis, notify])
}

function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return
      const key = e.key.toLowerCase()
      if (key === 's') {
        e.preventDefault()
        void (e.shiftKey ? saveProjectAs() : saveProject())
      } else if (key === 'o') {
        e.preventDefault()
        void openProject()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}

function Toast() {
  const notice = useStore((s) => s.notice)
  const notify = useStore((s) => s.notify)
  useEffect(() => {
    if (!notice) return
    const t = setTimeout(() => notify(null), notice.kind === 'error' ? 8000 : 3500)
    return () => clearTimeout(t)
  }, [notice, notify])
  if (!notice) return null
  return (
    <div className={`toast ${notice.kind}`} role="alert" onClick={() => notify(null)}>
      {notice.text}
    </div>
  )
}

export default function App() {
  const view = useStore((s) => s.view)
  const selection = useStore((s) => s.selection)
  // A selected route replaces the canvas by the endpoint configurator.
  const endpoint = view === 'architecture' && selection?.kind === 'route' ? selection : null
  useAnalysis()
  useShortcuts()

  // The first launch opens the demo project so the canvas is never blank.
  useEffect(() => {
    void api.sampleProject().then((p) => useStore.getState().load(p, null))
    void checkForUpdate({ silent: true })
  }, [])

  return (
    <div className={`app view-${endpoint ? 'endpoint' : view}`}>
      <Toolbar />
      {view === 'architecture' && <Palette />}
      <main className="center">
        <div className="stage">
          {view === 'tables' ? (
            <TablesView />
          ) : endpoint ? (
            <EndpointEditor service={endpoint.service} route={endpoint.route} />
          ) : view === 'architecture' ? (
            <Canvas />
          ) : (
            <CodeView />
          )}
        </div>
        <Problems />
      </main>
      {view === 'architecture' && !endpoint && (
        <aside className="inspector-panel" aria-label="Inspecteur">
          <Inspector />
        </aside>
      )}
      <Toast />
    </div>
  )
}

