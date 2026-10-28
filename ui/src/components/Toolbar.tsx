import { exportCode, loadSample, newProject, openProject, saveProject, saveProjectAs } from '../actions'
import { errorCount, useStore, warningCount } from '../store'
import { Icon } from './Icon'
import { CommitInput } from './ui'

export function Toolbar() {
  const project = useStore((s) => s.project)
  const dirty = useStore((s) => s.dirty)
  const filePath = useStore((s) => s.filePath)
  const view = useStore((s) => s.view)
  const setView = useStore((s) => s.setView)
  const edit = useStore((s) => s.edit)
  const analysis = useStore((s) => s.analysis)
  const errors = errorCount(analysis)
  const warnings = warningCount(analysis)

  return (
    <header className="toolbar">
      <div className="brand" aria-label="Service Builder">
        <img src="/icon.svg" alt="" width={26} height={26} />
        <strong>Service Builder</strong>
      </div>

      <div className="project-name">
        <CommitInput value={project.name} label="Nom du projet" onCommit={(v) => edit((p) => (p.name = v))} />
        {dirty && <span className="dirty" title="Modifications non enregistrées">●</span>}
        <small className="file mono" title={filePath ?? undefined}>{filePath ? filePath.split('/').pop() : 'non enregistré'}</small>
      </div>

      <div className="tabs" role="tablist">
        <button type="button" role="tab" aria-selected={view === 'architecture'} className={view === 'architecture' ? 'active' : ''} onClick={() => setView('architecture')}>
          <Icon name="graph" size={15} /> Architecture
        </button>
        <button type="button" role="tab" aria-selected={view === 'code'} className={view === 'code' ? 'active' : ''} onClick={() => setView('code')}>
          <Icon name="code" size={15} /> Code généré
        </button>
      </div>

      <span className="spacer" />

      <span className={`status ${errors > 0 ? 'error' : warnings > 0 ? 'warning' : 'ok'}`} role="status">
        <Icon name={errors > 0 ? 'error' : warnings > 0 ? 'warn' : 'ok'} size={14} />
        {errors > 0 ? `${errors} erreur${errors > 1 ? 's' : ''}` : warnings > 0 ? `${warnings} avertissement${warnings > 1 ? 's' : ''}` : 'Valide'}
      </span>

      <div className="btn-group">
        <button type="button" className="ghost" onClick={() => void newProject()} title="Nouveau projet vide"><Icon name="new" size={15} /><span className="label">Nouveau</span></button>
        <button type="button" className="ghost" onClick={() => void loadSample()} title="Charger le projet d’exemple"><Icon name="sparkles" size={15} /><span className="label">Exemple</span></button>
        <button type="button" className="ghost" onClick={() => void openProject()} title="Ouvrir (Ctrl+O)"><Icon name="open" size={15} /> Ouvrir</button>
        <button type="button" className="ghost" onClick={() => void saveProject()} title="Enregistrer (Ctrl+S)"><Icon name="save" size={15} /> Enregistrer</button>
        <button type="button" className="ghost" onClick={() => void saveProjectAs()} title="Enregistrer sous… (Ctrl+Maj+S)" aria-label="Enregistrer sous">
          <Icon name="copy" size={15} />
        </button>
      </div>
      <button type="button" className="primary" onClick={() => void exportCode()} disabled={errors > 0} title={errors > 0 ? 'Corrigez les erreurs avant d’exporter' : 'Générer le workspace Cargo'}>
        <Icon name="export" size={15} /> Exporter le code
      </button>
    </header>
  )
}
