import { selectionForPath } from '../model'
import { errorCount, useStore, warningCount } from '../store'
import { Icon } from './Icon'

export function Problems() {
  const analysis = useStore((s) => s.analysis)
  const project = useStore((s) => s.project)
  const open = useStore((s) => s.problemsOpen)
  const toggle = useStore((s) => s.toggleProblems)
  const select = useStore((s) => s.select)
  const setView = useStore((s) => s.setView)
  const errors = errorCount(analysis)
  const warnings = warningCount(analysis)
  const sorted = [...analysis.diagnostics].sort((a, b) => (a.severity === b.severity ? 0 : a.severity === 'error' ? -1 : 1))

  return (
    <section className="problems" aria-label="Problèmes">
      <button type="button" className="problems-head" onClick={toggle} aria-expanded={open}>
        <span className={`chev${open ? ' open' : ''}`}><Icon name="right" size={13} /></span>
        <strong>Problèmes</strong>
        {errors > 0 && <span className="count error"><Icon name="error" size={12} /> {errors}</span>}
        {warnings > 0 && <span className="count warning"><Icon name="warn" size={12} /> {warnings}</span>}
        {errors === 0 && warnings === 0 && <span className="count ok"><Icon name="ok" size={12} /> Aucun problème</span>}
      </button>
      {open && sorted.length > 0 && (
        <ul>
          {sorted.map((d, i) => (
            <li key={i}>
              <button
                type="button"
                className={d.severity}
                onClick={() => {
                  const sel = selectionForPath(project, d.path)
                  if (sel) {
                    select(sel)
                    setView('architecture')
                  }
                }}
              >
                <Icon name={d.severity === 'error' ? 'error' : 'warn'} size={13} />
                <span className="msg">{d.message}</span>
                <code>{d.code}</code>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
