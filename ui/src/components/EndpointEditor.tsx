import { useMemo } from 'react'
import { addMessage, defaultStatus, makeStep, METHODS, newId, pathParams, uniqueName } from '../model'
import { exampleJson, exampleQuery } from '../json'
import { inputScope, scopeAfter, scopesOf } from '../scope'
import { useStore } from '../store'
import type { Field, HttpMethod, Project, Route, Step } from '../types'
import { AssignmentsEditor } from './AssignmentsEditor'
import { Icon } from './Icon'
import { MessageSelect, SchemaFields } from './SchemaFields'
import { type HandlerCtx, StepList } from './StepList'
import { CommitInput, Empty, IconButton } from './ui'

/** Last Respond step at the top level of the route: the one that produces the answer. */
const lastRespond = (steps: Step[]) => steps.map((s) => s.type).lastIndexOf('respond')

function hasRespondInBranch(steps: Step[]): boolean {
  return steps.some((s) => s.type === 'branch' && [...s.then, ...s.otherwise].some((c) => c.type === 'respond' || hasRespondInBranch([c])))
}

/**
 * The endpoint configurator, in place of the architecture canvas: the logic of the route on the left,
 * the JSON it receives and answers on the right. The answer is produced by the logic: each field of the
 * response reads a value the steps computed.
 */
export function EndpointEditor(props: { service: string; route: string }) {
  const project = useStore((s) => s.project)
  const diagnostics = useStore((s) => s.analysis.diagnostics)
  const edit = useStore((s) => s.edit)
  const select = useStore((s) => s.select)

  const si = project.services.findIndex((s) => s.name === props.service)
  const service = project.services[si]
  const ri = service?.routes.findIndex((r) => r.name === props.route) ?? -1
  const route = service?.routes[ri]
  if (!service || !route) {
    return (
      <div className="endpoint">
        <div className="ep-empty">
          <Empty>Route introuvable.</Empty>
          <button type="button" className="link-btn" onClick={() => select(service ? { kind: 'service', name: service.name } : null)}>← Retour à l’architecture</button>
        </div>
      </div>
    )
  }
  return <Endpoint project={project} si={si} ri={ri} route={route} diagnostics={diagnostics} edit={edit} select={select} />
}

function Endpoint(props: {
  project: Project
  si: number
  ri: number
  route: Route
  diagnostics: { severity: 'error' | 'warning'; code: string; message: string; path: string }[]
  edit: (fn: (p: Project) => void) => void
  select: ReturnType<typeof useStore.getState>['select']
}) {
  const { project, si, ri, route, edit, select } = props
  const service = project.services[si]!
  const rt = (p: Project) => p.services[si]!.routes[ri]!
  const base = `/services/${si}/routes/${ri}`
  const ctx: HandlerCtx = { service: service.name, response: route.response, http: true }

  const params = pathParams(route.path)
  const origin = (f: Field) =>
    params.includes(f.name) ? 'chemin' : f.source ? `${f.source.from === 'header' ? 'en-tête' : 'cookie'} ${f.source.name}` : route.method === 'get' || route.method === 'delete' ? 'query' : 'corps'
  const requestIndex = project.messages.findIndex((m) => m.name === route.request)
  const responseIndex = project.messages.findIndex((m) => m.name === route.response)
  const response = project.messages[responseIndex]

  // The answer comes from the last top-level Respond; its scope holds every value the logic computed.
  const scopes = useMemo(() => scopesOf(project, inputScope(route.request), route.steps), [project, route.request, route.steps])
  const respondAt = lastRespond(route.steps)
  const respond = respondAt >= 0 ? (route.steps[respondAt] as Extract<Step, { type: 'respond' }>) : null
  const endScope = route.steps.length ? scopeAfter(project, scopes.at(-1)!, route.steps.at(-1)!) : inputScope(route.request)
  const respondScope = respond ? scopes[respondAt]! : endScope
  const nested = hasRespondInBranch(route.steps)
  // Variables holding a whole response, built by the logic.
  const wholes = Object.entries(respondScope).filter(([name, t]) => name !== 'input' && t.type.kind === 'message' && t.type.name === route.response)

  const mine = props.diagnostics.filter((d) => (d.path === base || d.path.startsWith(`${base}/`)) && !d.path.startsWith(`${base}/steps`))

  const ensureRespond = (p: Project) => {
    const steps = rt(p).steps
    let at = lastRespond(steps)
    if (at < 0) {
      steps.push(makeStep('respond', p, respondScope, ctx))
      at = steps.length - 1
    }
    return steps[at] as Extract<Step, { type: 'respond' }>
  }

  /** A variable of the response schema, to fill with Rust (queries over several tables…), then read field by field. */
  const buildByLogic = () =>
    edit((p) => {
      const steps = rt(p).steps
      const name = uniqueName('result', Object.keys(respondScope))
      const at = lastRespond(steps)
      const letStep: Step = {
        type: 'let',
        id: newId(),
        name,
        valueType: { kind: 'message', name: route.response },
        optional: false,
        repeated: false,
        value: {
          kind: 'raw',
          code: `{\n    // Interrogez vos tables avec \`tx\`, par exemple :\n    // let rows = sqlx::query("SELECT …").fetch_all(&mut *tx).await?;\n    pb::${route.response} { ..Default::default() }\n}`,
        },
      }
      steps.splice(at >= 0 ? at : steps.length, 0, letStep)
      const r = ensureRespond(p)
      r.fields = (p.messages.find((m) => m.name === route.response)?.fields ?? []).map((f) => ({ field: f.name, value: { kind: 'path' as const, path: `${name}.${f.name}` } }))
    })

  const readFrom = (variable: string) =>
    edit((p) => {
      const r = ensureRespond(p)
      r.fields = (p.messages.find((m) => m.name === route.response)?.fields ?? []).map((f) => ({ field: f.name, value: { kind: 'path' as const, path: `${variable}.${f.name}` } }))
    })

  const newSchema = (which: 'request' | 'response') =>
    edit((p) => {
      const m = addMessage(p, `${route.name.split('_').map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join('')}${which === 'request' ? 'Request' : 'Response'}`, 'api')
      rt(p)[which] = m.name
    })

  const hasBody = route.method === 'post' || route.method === 'put' || route.method === 'patch'
  const notPath = (f: Field) => !params.includes(f.name) && !f.source

  return (
    <div className="endpoint">
      <header className="ep-head">
        <button type="button" className="ep-back" onClick={() => select({ kind: 'service', name: service.name })} title="Retour au canvas d’architecture">
          ← Architecture
        </button>
        <span className="ep-service"><Icon name="service" size={14} /> {service.name}</span>
        <select
          className={`ep-method ${route.method}`}
          aria-label={`Méthode de ${route.name}`}
          value={route.method}
          onChange={(e) =>
            edit((p) => {
              const r = rt(p)
              const before = defaultStatus(r.method)
              r.method = e.target.value as HttpMethod
              if (r.status === before) r.status = defaultStatus(r.method)
            })
          }
        >
          {METHODS.map((m) => (
            <option key={m} value={m}>{m.toUpperCase()}</option>
          ))}
        </select>
        <div className="ep-path">
          <CommitInput value={route.path} mono label={`Chemin de ${route.name}`} onCommit={(v) => edit((p) => (rt(p).path = v))} />
        </div>
        <label className="ep-field">
          <span>Nom</span>
          <CommitInput
            value={route.name}
            mono
            label="Nom de la route"
            onCommit={(v) => {
              edit((p) => (rt(p).name = v))
              select({ kind: 'route', service: service.name, route: v })
            }}
          />
        </label>
        <label className="ep-field">
          <span>Statut</span>
          <select aria-label={`Statut de ${route.name}`} value={route.status} onChange={(e) => edit((p) => (rt(p).status = Number(e.target.value)))}>
            <option value={200}>200 OK</option>
            <option value={201}>201 Created</option>
            <option value={202}>202 Accepted</option>
          </select>
        </label>
        <label className="ep-field">
          <span>Port HTTP</span>
          <input
            type="number"
            aria-label="Port HTTP"
            value={service.httpPort ?? ''}
            min={1}
            onChange={(e) => edit((p) => (p.services[si]!.httpPort = e.target.value === '' ? null : Number(e.target.value)))}
          />
        </label>
        <span className="spacer" />
        <IconButton
          icon="trash"
          title={`Supprimer la route ${route.name}`}
          danger
          onClick={() => {
            edit((p) => p.services[si]!.routes.splice(ri, 1))
            select({ kind: 'service', name: service.name })
          }}
        />
      </header>

      {mine.length > 0 && (
        <ul className="ep-problems">
          {mine.map((d, i) => (
            <li key={i} className={d.severity}>
              <Icon name={d.severity === 'error' ? 'error' : 'warn'} size={13} /> <code>{d.code}</code> {d.message}
            </li>
          ))}
        </ul>
      )}

      <div className="ep-body">
        <section className="ep-col ep-logic" aria-label="Logique">
          <h3>Logique</h3>
          <p className="ep-hint">
            Les étapes s’exécutent dans une transaction. <code>input</code> est la requête ; la réponse est construite ici, puis
            renvoyée par l’étape « Répondre ».
          </p>
          <StepList steps={route.steps} base={`${base}/steps`} scope={inputScope(route.request)} ctx={ctx} locate={(p) => rt(p).steps} />
        </section>

        <section className="ep-col ep-contract" aria-label="Contrat JSON">
          <article className="ep-card" aria-label="Requête">
            <header>
              <h3>Requête</h3>
              <MessageSelect kind="api" value={route.request} onChange={(v) => edit((p) => (rt(p).request = v))} />
              <IconButton icon="plus" title="Nouveau schéma de requête" onClick={() => newSchema('request')} />
            </header>
            <p className="ep-hint">
              {params.length > 0 && <>Dans le chemin : {params.map((n) => <code key={n}>{`{${n}}`}</code>)}. </>}
              {hasBody ? 'Le reste vient du corps JSON.' : 'Le reste vient de la query string (champs simples).'}
            </p>
            {requestIndex >= 0 ? (
              <>
                <SchemaFields index={requestIndex} origin={origin} />
                <div className="json-preview" aria-label="Exemple de requête">
                  <div className="json-title">
                    <span className={`tag-pill http ${route.method}`}>{route.method.toUpperCase()}</span> <span className="mono">{route.path}</span>
                    {!hasBody && <span className="mono">{exampleQuery(project, route.request, notPath)}</span>}
                  </div>
                  {hasBody && <pre className="mono">{exampleJson(project, route.request, notPath)}</pre>}
                </div>
              </>
            ) : (
              <Empty>Choisissez un schéma API pour la requête.</Empty>
            )}
          </article>

          <article className="ep-card" aria-label="Réponse">
            <header>
              <h3>Réponse</h3>
              <MessageSelect kind="api" value={route.response} onChange={(v) => edit((p) => (rt(p).response = v))} />
              <IconButton icon="plus" title="Nouveau schéma de réponse" onClick={() => newSchema('response')} />
              <span className="status-code">{route.status}</span>
            </header>
            {responseIndex >= 0 && response ? (
              <>
                <SchemaFields index={responseIndex} />
                <div className="json-preview" aria-label="Exemple de réponse">
                  <div className="json-title">JSON renvoyé</div>
                  <pre className="mono">{exampleJson(project, route.response)}</pre>
                </div>

                <h4 className="ep-sub">Valeurs, produites par la logique</h4>
                {respond ? (
                  <AssignmentsEditor
                    message={route.response}
                    assignments={respond.fields}
                    scope={respondScope}
                    update={(fn) => edit((p) => fn(ensureRespond(p).fields))}
                  />
                ) : nested ? (
                  <Empty>La réponse est produite dans une condition : éditez-la dans la logique.</Empty>
                ) : (
                  <div className="ep-actions">
                    <Empty>Aucune étape « Répondre » : la route renverra un objet vide.</Empty>
                    <button type="button" className="link-btn" onClick={() => edit((p) => void ensureRespond(p))}>+ Ajouter l’étape « Répondre »</button>
                  </div>
                )}
                {(respond || !nested) && (
                  <div className="ep-actions">
                    {wholes.map(([name]) => (
                      <button key={name} type="button" className="link-btn" onClick={() => readFrom(name)}>
                        <Icon name="sparkles" size={13} /> Tout lire depuis <code>{name}</code>
                      </button>
                    ))}
                    <button type="button" className="link-btn" onClick={buildByLogic} title="Ajoute une variable du type de la réponse, remplie en Rust (requêtes sur plusieurs tables par exemple)">
                      <Icon name="code" size={13} /> Construire la réponse par la logique (Rust)
                    </button>
                  </div>
                )}
              </>
            ) : (
              <Empty>Choisissez un schéma API pour la réponse.</Empty>
            )}
          </article>
        </section>
      </div>
    </div>
  )
}
