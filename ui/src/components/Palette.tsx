import { addMessage, addMtaService, addService, addTopic, KIND_LABELS, KINDS } from '../model'
import { useStore } from '../store'
import { DND_TYPE } from './Canvas'
import { Icon, type IconName } from './Icon'
import { IconButton } from './ui'

function Tile(props: { kind: 'service' | 'topic'; icon: IconName; label: string; hint: string; onAdd: () => void; draggable?: boolean }) {
  const draggable = props.draggable !== false
  return (
    <div
      className={`tile ${props.kind}`}
      draggable={draggable}
      onDragStart={(e) => {
        e.dataTransfer.setData(DND_TYPE, props.kind)
        e.dataTransfer.effectAllowed = 'copy'
      }}
      onDoubleClick={props.onAdd}
      title={draggable ? 'Glisser sur le canvas, ou double-clic pour ajouter' : 'Double-clic pour ajouter'}
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

  // On an endpoint only services, routes and API schemas matter: events, proto schemas and topics are out of the way.
  const apiMode = selection?.kind === 'route'
  const spot = () => ({ x: 80 + project.services.length * 40, y: 60 + project.topics.length * 40 })
  const add = (kind: 'service' | 'topic') => {
    let name = ''
    edit((p) => {
      name = kind === 'service' ? addService(p, spot()).name : addTopic(p, spot()).name
    })
    select({ kind, name })
  }

  const addMta = () => {
    let name = ''
    edit((p) => {
      name = addMtaService(p, spot()).name
    })
    select({ kind: 'service', name })
  }

  return (
    <aside className="palette" aria-label="Éléments">
      {!apiMode && (
        <>
          <h2>Éléments</h2>
          <Tile kind="service" icon="service" label="Service" hint="gRPC, HTTP + consumers Kafka" onAdd={() => add('service')} />
          <Tile kind="service" icon="publish" label="Service mail" hint="MTA prêt à l’emploi : événements → mails" onAdd={addMta} draggable={false} />
          <Tile kind="topic" icon="topic" label="Topic" hint="Flux d’événements Kafka" onAdd={() => add('topic')} />
        </>
      )}

      <h2>Structure</h2>
      <nav className="tree">
        <h4>Services</h4>
        {project.services.length === 0 && <p className="empty">Aucun service</p>}
        {project.services.map((s) => (
          <div key={s.name}>
            <button
              type="button"
              className={selection?.kind === 'service' && selection.name === s.name ? 'active' : ''}
              onClick={() => select({ kind: 'service', name: s.name })}
            >
              <Icon name="service" size={13} /> {s.name}
            </button>
            {s.routes.map((r) => (
              <button
                key={r.name}
                type="button"
                className={`sub${selection?.kind === 'route' && selection.service === s.name && selection.route === r.name ? ' active' : ''}`}
                onClick={() => select({ kind: 'route', service: s.name, route: r.name })}
                title={`${r.method.toUpperCase()} ${r.path}`}
              >
                <span className={`tag-pill http ${r.method}`}>{r.method.toUpperCase()}</span> <span className="mono route-path">{r.path}</span>
              </button>
            ))}
          </div>
        ))}
        {!apiMode && (
          <>
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
          </>
        )}
        {KINDS.filter((kind) => !apiMode || kind === 'api').map((kind) => {
          const list = project.messages.filter((m) => m.kind === kind)
          return (
            <section key={kind} aria-label={`Schémas ${KIND_LABELS[kind].title}`}>
              <h4 title={KIND_LABELS[kind].hint}>
                <span>
                  {KIND_LABELS[kind].short}
                </span>
                <IconButton
                  icon="plus"
                  title={`Ajouter un schéma ${KIND_LABELS[kind].one}`}
                  size={13}
                  onClick={() => {
                    let n = ''
                    edit((p) => {
                      n = addMessage(p, undefined, kind).name
                    })
                    select({ kind: 'messages', name: n })
                  }}
                />
              </h4>
              {list.length === 0 && <p className="empty">Aucun schéma</p>}
              {list.map((m) => (
                <button
                  key={m.name}
                  type="button"
                  className={selection?.kind === 'messages' && selection.name === m.name ? 'active' : ''}
                  onClick={() => select({ kind: 'messages', name: m.name })}
                >
                  <Icon name="schema" size={13} /> <span className="mono">{m.name}</span>
                </button>
              ))}
            </section>
          )
        })}
      </nav>
    </aside>
  )
}
