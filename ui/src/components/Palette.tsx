import { addMessage, addService, addTopic } from '../model'
import { useStore } from '../store'
import { DND_TYPE } from './Canvas'
import { Icon, type IconName } from './Icon'
import { IconButton } from './ui'

function Tile(props: { kind: 'service' | 'topic'; icon: IconName; label: string; hint: string; onAdd: () => void }) {
  return (
    <div
      className={`tile ${props.kind}`}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData(DND_TYPE, props.kind)
        e.dataTransfer.effectAllowed = 'copy'
      }}
      onDoubleClick={props.onAdd}
      title="Glisser sur le canvas, ou double-clic pour ajouter"
    >
      <Icon name="grip" size={14} />
      <span className="tile-icon"><Icon name={props.icon} size={18} /></span>
      <span className="tile-text">
        <strong>{props.label}</strong>
        <small>{props.hint}</small>
      </span>
      <IconButton icon="plus" title={`Ajouter : ${props.label}`} onClick={props.onAdd} />
    </div>
  )
}

export function Palette() {
  const project = useStore((s) => s.project)
  const selection = useStore((s) => s.selection)
  const edit = useStore((s) => s.edit)
  const select = useStore((s) => s.select)

  const spot = () => ({ x: 80 + project.services.length * 40, y: 60 + project.topics.length * 40 })
  const add = (kind: 'service' | 'topic') => {
    let name = ''
    edit((p) => {
      name = kind === 'service' ? addService(p, spot()).name : addTopic(p, spot()).name
    })
    select({ kind, name })
  }

  return (
    <aside className="palette" aria-label="Éléments">
      <h2>Éléments</h2>
      <Tile kind="service" icon="service" label="Service" hint="gRPC + consumers Kafka" onAdd={() => add('service')} />
      <Tile kind="topic" icon="topic" label="Topic" hint="Flux d’événements Kafka" onAdd={() => add('topic')} />

      <h2>Structure</h2>
      <nav className="tree">
        <h4>Services</h4>
        {project.services.length === 0 && <p className="empty">Aucun service</p>}
        {project.services.map((s) => (
          <button
            key={s.name}
            type="button"
            className={selection?.kind === 'service' && selection.name === s.name ? 'active' : ''}
            onClick={() => select({ kind: 'service', name: s.name })}
          >
            <Icon name="service" size={13} /> {s.name}
          </button>
        ))}
        <h4>Topics</h4>
        {project.topics.length === 0 && <p className="empty">Aucun topic</p>}
        {project.topics.map((t) => (
          <button
            key={t.name}
            type="button"
            className={selection?.kind === 'topic' && selection.name === t.name ? 'active' : ''}
            onClick={() => select({ kind: 'topic', name: t.name })}
          >
            <Icon name="topic" size={13} /> <span className="mono">{t.name}</span>
          </button>
        ))}
        <h4>
          Schémas
          <IconButton icon="plus" title="Ajouter un message" size={13} onClick={() => { let n = ''; edit((p) => { n = addMessage(p).name }); select({ kind: 'messages', name: n }) }} />
        </h4>
        {project.messages.length === 0 && <p className="empty">Aucun message</p>}
        {project.messages.map((m) => (
          <button
            key={m.name}
            type="button"
            className={selection?.kind === 'messages' && selection.name === m.name ? 'active' : ''}
            onClick={() => select({ kind: 'messages', name: m.name })}
          >
            <Icon name="schema" size={13} /> <span className="mono">{m.name}</span>
          </button>
        ))}
      </nav>
    </aside>
  )
}
