import { addConsumer, addRpc, servicesUsingTopic, renameField, renameMessage, renameRpc, renameService, renameTopic, addMessage, nextTag } from '../model'
import { inputScope, SCALARS, typeOfField } from '../scope'
import { useStore } from '../store'
import type { Field, FieldType, Project } from '../types'
import { type HandlerCtx, StepList } from './StepList'
import { Collapsible, CommitInput, Empty, IconButton, NumberInput, Row, Section } from './ui'

export function Inspector() {
  const selection = useStore((s) => s.selection)
  switch (selection?.kind) {
    case 'service':
      return <ServiceInspector name={selection.name} />
    case 'topic':
      return <TopicInspector name={selection.name} />
    case 'messages':
      return <MessagesInspector focus={selection.name} />
    default:
      return (
        <div className="inspector-empty">
          <h3>Rien de sélectionné</h3>
          <p>Cliquez sur un service ou un topic dans le canvas pour configurer ses RPC, ses consumers et leur logique.</p>
          <p>Glissez un <strong>Service</strong> ou un <strong>Topic</strong> depuis la colonne de gauche pour construire votre architecture.</p>
        </div>
      )
  }
}

// ---- service ----------------------------------------------------------------------

function ServiceInspector({ name }: { name: string }) {
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  const select = useStore((s) => s.select)
  const index = project.services.findIndex((s) => s.name === name)
  const service = project.services[index]
  if (!service) return <Empty>Service introuvable.</Empty>
  const svc = (p: Project) => p.services[index]!

  return (
    <div className="inspector">
      <Section title="Service" icon="service">
        <Row label="Nom">
          <CommitInput value={service.name} mono label="Nom du service" onCommit={(v) => { edit((p) => renameService(p, service.name, v)); select({ kind: 'service', name: v }) }} />
        </Row>
        <Row label="Port gRPC">
          <NumberInput value={service.port} min={1} label="Port" onChange={(v) => edit((p) => (svc(p).port = v))} />
        </Row>
      </Section>

      <Section title="RPC (gRPC)" icon="callRpc" action={<IconButton icon="plus" title="Ajouter un RPC" onClick={() => edit((p) => addRpc(p, svc(p)))} />}>
        {service.rpcs.length === 0 && <Empty>Aucun RPC : ce service n’expose pas d’API gRPC.</Empty>}
        {service.rpcs.map((rpc, ri) => {
          const ctx: HandlerCtx = { service: service.name, response: rpc.response }
          return (
            <Collapsible
              key={ri}
              tone="rpc"
              defaultOpen={service.rpcs.length === 1}
              title={<><strong className="mono">{rpc.name}</strong><span className="step-summary">{rpc.request} → {rpc.response}</span></>}
              actions={<IconButton icon="trash" title={`Supprimer ${rpc.name}`} danger onClick={() => edit((p) => svc(p).rpcs.splice(ri, 1))} />}
            >
              <Row label="Nom">
                <CommitInput value={rpc.name} mono label="Nom du RPC" onCommit={(v) => edit((p) => renameRpc(p, service.name, rpc.name, v))} />
              </Row>
              <Row label="Requête">
                <MessageSelect value={rpc.request} onChange={(v) => edit((p) => (svc(p).rpcs[ri]!.request = v))} />
              </Row>
              <Row label="Réponse">
                <MessageSelect value={rpc.response} onChange={(v) => edit((p) => (svc(p).rpcs[ri]!.response = v))} />
              </Row>
              <h4 className="steps-title">Logique</h4>
              <StepList
                steps={rpc.steps}
                base={`/services/${index}/rpcs/${ri}/steps`}
                scope={inputScope(rpc.request)}
                ctx={ctx}
                locate={(p) => svc(p).rpcs[ri]!.steps}
              />
            </Collapsible>
          )
        })}
      </Section>

      <Section
        title="Consumers (Kafka)"
        icon="topic"
        action={<IconButton icon="plus" title={project.topics.length ? 'Ajouter un consumer' : 'Créez d’abord un topic'} disabled={project.topics.length === 0} onClick={() => edit((p) => addConsumer(p, svc(p)))} />}
      >
        {service.consumers.length === 0 && <Empty>Aucun consumer : ce service ne réagit à aucun événement.</Empty>}
        {service.consumers.map((c, ci) => {
          const topic = project.topics.find((t) => t.name === c.topic)
          const ctx: HandlerCtx = { service: service.name, response: null }
          return (
            <Collapsible
              key={ci}
              tone="consumer"
              defaultOpen={service.consumers.length === 1}
              title={<><strong className="mono">{c.name}</strong><span className="step-summary">⇠ {c.topic}</span></>}
              actions={<IconButton icon="trash" title={`Supprimer ${c.name}`} danger onClick={() => edit((p) => svc(p).consumers.splice(ci, 1))} />}
            >
              <Row label="Nom">
                <CommitInput value={c.name} mono label="Nom du consumer" onCommit={(v) => edit((p) => (svc(p).consumers[ci]!.name = v))} />
              </Row>
              <Row label="Topic">
                <select aria-label="Topic écouté" value={c.topic} onChange={(e) => edit((p) => (svc(p).consumers[ci]!.topic = e.target.value))}>
                  {!topic && <option value={c.topic}>{c.topic}</option>}
                  {project.topics.map((t) => (
                    <option key={t.name}>{t.name}</option>
                  ))}
                </select>
              </Row>
              <Row label="Groupe" hint="Par défaut : service-consumer">
                <input
                  className="mono"
                  aria-label="Groupe Kafka"
                  value={c.groupId ?? ''}
                  placeholder={`${service.name.toLowerCase()}-${c.name}`}
                  spellCheck={false}
                  onChange={(e) => edit((p) => (svc(p).consumers[ci]!.groupId = e.target.value === '' ? null : e.target.value))}
                />
              </Row>
              <h4 className="steps-title">Logique</h4>
              {topic ? (
                <StepList
                  steps={c.steps}
                  base={`/services/${index}/consumers/${ci}/steps`}
                  scope={inputScope(topic.message)}
                  ctx={ctx}
                  locate={(p) => svc(p).consumers[ci]!.steps}
                />
              ) : (
                <Empty>Choisissez un topic existant pour éditer la logique.</Empty>
              )}
            </Collapsible>
          )
        })}
      </Section>

      <button
        type="button"
        className="danger-btn"
        onClick={() => {
          edit((p) => p.services.splice(index, 1))
          select(null)
        }}
      >
        Supprimer le service
      </button>
    </div>
  )
}

function MessageSelect(props: { value: string; onChange: (v: string) => void }) {
  const messages = useStore((s) => s.project.messages)
  return (
    <select aria-label="Message" value={props.value} onChange={(e) => props.onChange(e.target.value)}>
      {!messages.some((m) => m.name === props.value) && <option value={props.value}>{props.value || 'Choisir…'}</option>}
      {messages.map((m) => (
        <option key={m.name}>{m.name}</option>
      ))}
    </select>
  )
}

// ---- topic ------------------------------------------------------------------------

function TopicInspector({ name }: { name: string }) {
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  const select = useStore((s) => s.select)
  const index = project.topics.findIndex((t) => t.name === name)
  const topic = project.topics[index]
  if (!topic) return <Empty>Topic introuvable.</Empty>
  const msg = project.messages.find((m) => m.name === topic.message)
  const keyable = (msg?.fields ?? []).filter((f) => {
    const t = typeOfField(f)
    return !t.repeated && !t.optional && ['string', 'int32', 'int64', 'bool'].includes(t.type.kind)
  })
  const users = servicesUsingTopic(project, topic.name)

  return (
    <div className="inspector">
      <Section title="Topic Kafka" icon="topic">
        <Row label="Nom" hint="Lettres, chiffres, . _ -">
          <CommitInput value={topic.name} mono label="Nom du topic" onCommit={(v) => { edit((p) => renameTopic(p, topic.name, v)); select({ kind: 'topic', name: v }) }} />
        </Row>
        <Row label="Événement" hint="Message protobuf transporté">
          <MessageSelect value={topic.message} onChange={(v) => edit((p) => { const t = p.topics[index]!; t.message = v; t.keyField = null })} />
        </Row>
        <Row label="Clé" hint="Les événements de même clé restent ordonnés">
          <select aria-label="Champ clé" value={topic.keyField ?? ''} onChange={(e) => edit((p) => (p.topics[index]!.keyField = e.target.value === '' ? null : e.target.value))}>
            <option value="">aucune</option>
            {topic.keyField && !keyable.some((f) => f.name === topic.keyField) && <option value={topic.keyField}>{topic.keyField} (invalide)</option>}
            {keyable.map((f) => (
              <option key={f.name}>{f.name}</option>
            ))}
          </select>
        </Row>
        <Row label="Partitions">
          <NumberInput value={topic.partitions} min={1} label="Partitions" onChange={(v) => edit((p) => (p.topics[index]!.partitions = v))} />
        </Row>
        <Row label="Réplication">
          <NumberInput value={topic.replicationFactor} min={1} label="Facteur de réplication" onChange={(v) => edit((p) => (p.topics[index]!.replicationFactor = v))} />
        </Row>
      </Section>
      <Section title="Utilisé par" icon="service">
        {users.length === 0 ? <Empty>Aucun service ne publie ni ne consomme ce topic.</Empty> : (
          <ul className="plain-list">
            {users.map((s) => (
              <li key={s.name}>
                <button type="button" className="link-btn" onClick={() => select({ kind: 'service', name: s.name })}>{s.name}</button>
              </li>
            ))}
          </ul>
        )}
      </Section>
      <button type="button" className="danger-btn" onClick={() => { edit((p) => p.topics.splice(index, 1)); select(null) }}>
        Supprimer le topic
      </button>
    </div>
  )
}

// ---- messages ---------------------------------------------------------------------

const TYPE_OPTIONS: string[] = [...SCALARS, 'bytes', 'timestamp']

function typeValue(t: FieldType): string {
  return t.kind === 'message' ? `msg:${t.name}` : t.kind
}

function parseType(v: string): FieldType {
  return v.startsWith('msg:') ? { kind: 'message', name: v.slice(4) } : ({ kind: v } as FieldType)
}

function MessagesInspector({ focus }: { focus?: string }) {
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  return (
    <div className="inspector">
      <Section title="Schémas (protobuf)" icon="schema" action={<IconButton icon="plus" title="Ajouter un message" onClick={() => edit((p) => addMessage(p))} />}>
        {project.messages.length === 0 && <Empty>Aucun message. Les messages décrivent les événements Kafka et les requêtes/réponses gRPC.</Empty>}
        {project.messages.map((m, mi) => (
          <Collapsible
            key={m.name}
            defaultOpen={focus ? focus === m.name : project.messages.length <= 3}
            title={<><strong className="mono">{m.name}</strong><span className="step-summary">{m.fields.length} champ{m.fields.length > 1 ? 's' : ''}</span></>}
            actions={<IconButton icon="trash" title={`Supprimer ${m.name}`} danger onClick={() => edit((p) => p.messages.splice(mi, 1))} />}
          >
            <Row label="Nom">
              <CommitInput value={m.name} mono label="Nom du message" onCommit={(v) => edit((p) => renameMessage(p, m.name, v))} />
            </Row>
            <div className="fields">
              {m.fields.map((f, fi) => (
                <FieldRow
                  key={fi}
                  field={f}
                  index={fi}
                  tag={f.tag ?? fi + 1}
                  onRename={(v) => edit((p) => renameField(p, m.name, f.name, v))}
                  onChange={(fn) => edit((p) => fn(p.messages[mi]!.fields[fi]!))}
                  onDelete={() => edit((p) => p.messages[mi]!.fields.splice(fi, 1))}
                />
              ))}
            </div>
            <button
              type="button"
              className="link-btn"
              onClick={() =>
                edit((p) => {
                  const msg = p.messages[mi]!
                  const used = msg.fields.map((x) => x.name)
                  let n = msg.fields.length + 1
                  while (used.includes(`field_${n}`)) n++
                  msg.fields.push({ name: `field_${n}`, type: { kind: 'string' }, repeated: false, optional: false, tag: nextTag(msg) })
                })
              }
            >
              + Ajouter un champ
            </button>
          </Collapsible>
        ))}
      </Section>
    </div>
  )
}

function FieldRow(props: {
  field: Field
  index: number
  tag: number
  onRename: (v: string) => void
  onChange: (fn: (f: Field) => void) => void
  onDelete: () => void
}) {
  const messages = useStore((s) => s.project.messages)
  const f = props.field
  const isMsg = f.type.kind === 'message'
  return (
    <div className="field-row">
      <span className="tag" title="Numéro de champ protobuf, stable">#{props.tag}</span>
      <CommitInput value={f.name} mono label="Nom du champ" onCommit={props.onRename} />
      <select aria-label="Type du champ" value={typeValue(f.type)} onChange={(e) => props.onChange((x) => { x.type = parseType(e.target.value); if (x.type.kind === 'message') x.optional = false })}>
        <optgroup label="Types simples">
          {TYPE_OPTIONS.map((k) => (
            <option key={k} value={k}>{k}</option>
          ))}
        </optgroup>
        {messages.length > 0 && (
          <optgroup label="Messages">
            {messages.map((m) => (
              <option key={m.name} value={`msg:${m.name}`}>{m.name}</option>
            ))}
          </optgroup>
        )}
        {f.type.kind === 'message' && !messages.some((m) => m.name === (f.type as { name: string }).name) && (
          <option value={typeValue(f.type)}>{(f.type as { name: string }).name} (inconnu)</option>
        )}
      </select>
      <label className="check" title="Liste">
        <input type="checkbox" checked={f.repeated} onChange={(e) => props.onChange((x) => { x.repeated = e.target.checked; if (e.target.checked) x.optional = false })} />
        []
      </label>
      <label className="check" title="Optionnel (présence suivie)">
        <input type="checkbox" checked={f.optional} disabled={isMsg || f.repeated} onChange={(e) => props.onChange((x) => (x.optional = e.target.checked))} />
        ?
      </label>
      <IconButton icon="trash" title={`Supprimer ${f.name}`} danger onClick={props.onDelete} />
    </div>
  )
}
