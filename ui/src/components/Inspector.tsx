import { addConsumer, addJob, addRoute, addRpc, consumersOf, declareProducer, KIND_LABELS, KINDS, removeService, renameMessage, renameRpc, renameService, renameTopic, addMessage } from '../model'
import { inputScope, typeOfField } from '../scope'
import { useStore } from '../store'
import type { MessageKind, Project } from '../types'
import { MessageSelect, SchemaFields } from './SchemaFields'
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
      return <ProjectInspector />
  }
}

// ---- project ----------------------------------------------------------------------

function ProjectInspector() {
  const attempts = useStore((s) => s.project.outbox?.maxAttempts ?? 5)
  const edit = useStore((s) => s.edit)
  return (
    <>
      <div className="inspector-empty">
        <h3>Rien de sélectionné</h3>
        <p>Cliquez sur un service ou un topic dans le canvas pour configurer ses RPC, ses consumers et leur logique.</p>
        <p>Glissez un <strong>Service</strong> ou un <strong>Topic</strong> depuis la colonne de gauche pour construire votre architecture.</p>
      </div>
      <div className="inspector">
        <Section title="Publication des événements" icon="publish">
          <Row label="Tentatives" hint="Chaque événement publié passe par l’outbox du service (même transaction que le flow). Si Kafka le refuse après ce nombre de tentatives, il est déplacé sur <topic>.dlq.">
            <NumberInput value={attempts} min={1} step={1} label="Tentatives avant DLQ" onChange={(v) => edit((p) => (p.outbox = { maxAttempts: Math.min(100, Math.max(1, Math.round(v))) }))} />
          </Row>
        </Section>
      </div>
    </>
  )
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
        {service.routes.length > 0 && (
          <Row label="Port HTTP" hint="Port de l’API HTTP (les routes ci-dessous)">
            <NumberInput value={service.httpPort ?? 0} min={1} label="Port HTTP" onChange={(v) => edit((p) => (svc(p).httpPort = v))} />
          </Row>
        )}
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
                <MessageSelect kind="proto" value={rpc.request} onChange={(v) => edit((p) => (svc(p).rpcs[ri]!.request = v))} />
              </Row>
              <Row label="Réponse">
                <MessageSelect kind="proto" value={rpc.response} onChange={(v) => edit((p) => (svc(p).rpcs[ri]!.response = v))} />
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
        title="API HTTP"
        icon="publish"
        action={
          <IconButton
            icon="plus"
            title="Ajouter une route HTTP"
            onClick={() => {
              let route = ''
              edit((p) => {
                route = addRoute(p, svc(p)).name
              })
              select({ kind: 'route', service: service.name, route })
            }}
          />
        }
      >
        {service.routes.length === 0 && <Empty>Aucune route : ce service ne reçoit aucune requête HTTP du front.</Empty>}
        <ul className="plain-list routes">
          {service.routes.map((route, ri) => (
            <li key={ri} className="route-item">
              <button type="button" className="route-open" title="Ouvrir le configurateur d’endpoint" onClick={() => select({ kind: 'route', service: service.name, route: route.name })}>
                <span className={`tag-pill http ${route.method}`}>{route.method.toUpperCase()}</span>
                <strong className="mono route-path">{route.path}</strong>
                <span className="step-summary">{route.steps.length} étape{route.steps.length > 1 ? 's' : ''}</span>
              </button>
              <IconButton icon="trash" title={`Supprimer la route ${route.name}`} danger onClick={() => edit((p) => svc(p).routes.splice(ri, 1))} />
            </li>
          ))}
        </ul>
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

      <Section
        title="Tâches planifiées"
        icon="update"
        action={<IconButton icon="plus" title="Ajouter une tâche planifiée" onClick={() => edit((p) => addJob(svc(p)))} />}
      >
        {service.jobs.length === 0 && <Empty>Aucune tâche : rien ne s’exécute ici sans requête ni événement.</Empty>}
        {service.jobs.map((job, ji) => (
          <Collapsible
            key={ji}
            tone="consumer"
            title={<><strong className="mono">{job.name}</strong><span className="step-summary">toutes les {everyLabel(job.everySecs)}</span></>}
            actions={<IconButton icon="trash" title={`Supprimer la tâche ${job.name}`} danger onClick={() => edit((p) => svc(p).jobs.splice(ji, 1))} />}
          >
            <Row label="Nom">
              <CommitInput value={job.name} mono label="Nom de la tâche" onCommit={(v) => edit((p) => (svc(p).jobs[ji]!.name = v))} />
            </Row>
            <Row label="Toutes les" hint="En secondes, de 1 à 2 592 000 (30 jours). Un premier passage a lieu au démarrage.">
              <NumberInput label="Période de la tâche (secondes)" min={1} value={job.everySecs} onChange={(n) => edit((p) => (svc(p).jobs[ji]!.everySecs = n))} />
            </Row>
            <small className="muted">
              Chaque instance du service exécute la tâche : ses étapes doivent pouvoir se répéter sans dommage (un <code>DELETE</code> de ce qui a
              expiré, par exemple). Pas de <code>input</code> ici, et personne à qui répondre.
            </small>
            <h4 className="steps-title">Logique</h4>
            <StepList
              steps={job.steps}
              base={`/services/${index}/jobs/${ji}/steps`}
              scope={{}}
              ctx={{ service: service.name, response: null }}
              locate={(p) => svc(p).jobs[ji]!.steps}
            />
          </Collapsible>
        ))}
      </Section>

      <button
        type="button"
        className="danger-btn"
        onClick={() => {
          edit((p) => removeService(p, service.name))
          select(null)
        }}
      >
        Supprimer le service
      </button>
    </div>
  )
}

/** `420` → `7 min`, `3600` → `1 h`: the period of a job, as a person says it. */
export function everyLabel(secs: number): string {
  if (secs % 86400 === 0) return `${secs / 86400} j`
  if (secs % 3600 === 0) return `${secs / 3600} h`
  if (secs % 60 === 0) return `${secs / 60} min`
  return `${secs} s`
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
  const consumers = consumersOf(project, topic.name)
  const candidates = project.services.filter((s) => !topic.producers.includes(s.name))

  return (
    <div className="inspector">
      <Section title="Topic Kafka" icon="topic">
        <Row label="Nom" hint="Lettres, chiffres, . _ -">
          <CommitInput value={topic.name} mono label="Nom du topic" onCommit={(v) => { edit((p) => renameTopic(p, topic.name, v)); select({ kind: 'topic', name: v }) }} />
        </Row>
        <Row label="Événement" hint="Message protobuf transporté">
          <MessageSelect kind="event" value={topic.message} onChange={(v) => edit((p) => { const t = p.topics[index]!; t.message = v; t.keyField = null })} />
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
      <Section title="Producteurs" icon="publish">
        {topic.producers.length === 0 && <Empty>Aucun service ne produit ce topic : tirez une flèche d’un service vers lui, ou ajoutez-en un ci-dessous.</Empty>}
        <ul className="plain-list">
          {topic.producers.map((name) => (
            <li key={name} className="producer">
              <button type="button" className="link-btn" onClick={() => select({ kind: 'service', name })}>{name}</button>
              <IconButton icon="x" title={`Retirer ${name} des producteurs`} danger onClick={() => edit((p) => (p.topics[index]!.producers = p.topics[index]!.producers.filter((x) => x !== name)))} />
            </li>
          ))}
        </ul>
        {candidates.length > 0 && (
          <select aria-label="Ajouter un producteur" value="" onChange={(e) => e.target.value && edit((p) => declareProducer(p, e.target.value, topic.name))}>
            <option value="">+ Ajouter un producteur…</option>
            {candidates.map((s) => (
              <option key={s.name}>{s.name}</option>
            ))}
          </select>
        )}
      </Section>
      <Section title="Consommé par" icon="service">
        {consumers.length === 0 ? <Empty>Aucun service ne consomme ce topic : tirez une flèche du topic vers un service.</Empty> : (
          <ul className="plain-list">
            {consumers.map((s) => (
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

function MessagesInspector({ focus }: { focus?: string }) {
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  return (
    <div className="inspector">
      {KINDS.map((kind) => {
        const entries = project.messages.map((m, mi) => ({ m, mi })).filter((e) => e.m.kind === kind)
        return (
          <Section
            key={kind}
            title={KIND_LABELS[kind].title}
            icon="schema"
            action={<IconButton icon="plus" title={`Ajouter un schéma ${KIND_LABELS[kind].one}`} onClick={() => edit((p) => addMessage(p, undefined, kind))} />}
          >
            <p className="kind-hint">{KIND_LABELS[kind].hint}</p>
            {entries.length === 0 && <Empty>Aucun schéma de ce type.</Empty>}
            {entries.map(({ m, mi }) => (
              <Collapsible
                key={m.name}
                defaultOpen={focus ? focus === m.name : project.messages.length <= 3}
                title={<><strong className="mono">{m.name}</strong><span className="step-summary">{m.fields.length} champ{m.fields.length > 1 ? 's' : ''}</span></>}
                actions={<IconButton icon="trash" title={`Supprimer ${m.name}`} danger onClick={() => edit((p) => p.messages.splice(mi, 1))} />}
              >
                <Row label="Nom">
                  <CommitInput value={m.name} mono label="Nom du schéma" onCommit={(v) => edit((p) => renameMessage(p, m.name, v))} />
                </Row>
                <Row label="Type" hint={KIND_LABELS[m.kind].hint}>
                  <select aria-label={`Type de ${m.name}`} value={m.kind} onChange={(e) => edit((p) => (p.messages[mi]!.kind = e.target.value as MessageKind))}>
                    {KINDS.map((k) => (
                      <option key={k} value={k}>{KIND_LABELS[k].one}</option>
                    ))}
                  </select>
                </Row>
                <SchemaFields index={mi} />
              </Collapsible>
            ))}
          </Section>
        )
      })}
    </div>
  )
}
