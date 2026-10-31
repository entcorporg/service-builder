import { useState } from 'react'
import { declareProducer, ensureJwksSchemas, ERROR_STATUSES, exprText, makeStep, newId, STEP_LABELS } from '../model'
import { candidates, isSimpleScalar, SCALARS, type Scope, scopeAfter, scopesOf, scalar } from '../scope'
import { useStore } from '../store'
import type { CmpOp, Cookie, Condition, Diagnostic, FieldType, LogLevel, Project, SameSite, Step, StepType } from '../types'
import { AssignmentsEditor } from './AssignmentsEditor'
import { ExprEditor } from './ExprEditor'
import { Icon } from './Icon'
import { JwksBody, SignJwtBody, VerifyJwtBody } from './JwtSteps'
import { OP_LABELS, SqlBody } from './SqlStep'
import { Badge, Collapsible, Empty, IconButton, NumberInput } from './ui'

export interface HandlerCtx {
  service: string
  /** Response message when the handler is an RPC or an HTTP route, null for consumers. */
  response: string | null
  /** An HTTP route, which can end early with a `fail` status. */
  http?: boolean
}

interface ListProps {
  steps: Step[]
  /** JSON pointer of this list, to match diagnostics. */
  base: string
  /** Scope at the start of the list. */
  scope: Scope
  ctx: HandlerCtx
  /** Resolves this list inside a project draft. */
  locate: (p: Project) => Step[]
}

/** The literal a new variable starts with: valid for its type, so that the project stays valid while it is edited. */
function literalDefault(kind: string, repeated: boolean): string | number | boolean | unknown[] {
  if (repeated) return []
  if (kind === 'bool') return false
  if (kind === 'string') return ''
  if (kind === 'uuid') return '00000000-0000-0000-0000-000000000000'
  return 0
}


const STEP_ORDER: StepType[] = ['publish', 'sql', 'callRpc', 'branch', 'let', 'log', 'respond', 'fail', 'rateLimit', 'verifyHmac', 'signJwt', 'verifyJwt', 'jwks', 'encrypt', 'decrypt', 'hashPassword', 'verifyPassword', 'randomToken', 'sha256', 'code']

export function StepList({ steps, base, scope, ctx, locate }: ListProps) {
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  const diagnostics = useStore((s) => s.analysis.diagnostics)
  const scopes = scopesOf(project, scope, steps)
  const endScope = steps.length > 0 ? scopeAfter(project, scopes.at(-1)!, steps.at(-1)!) : scope
  const mutate = (fn: (list: Step[]) => void) => edit((p) => fn(locate(p)))

  return (
    <div className="steps">
      {steps.length === 0 && <Empty>Aucune étape pour l’instant.</Empty>}
      {steps.map((step, i) => (
        <StepCard
          key={step.id}
          step={step}
          index={i}
          count={steps.length}
          path={`${base}/${i}`}
          scope={scopes[i]!}
          ctx={ctx}
          diagnostics={diagnostics}
          mutate={mutate}
          locate={locate}
        />
      ))}
      <AddStep
        allowRespond={ctx.response !== null}
        allowFail={ctx.http === true}
        // The signature is checked before the body is read: the first step of the route itself, once.
        allowSignature={ctx.http === true && /\/routes\/\d+\/steps$/.test(base) && !steps.some((s) => s.type === 'verifyHmac')}
        onAdd={(type) =>
          edit((p) => {
            const step = makeStep(type, p, endScope, ctx)
            // A signature is checked before anything else runs.
            if (step.type === 'jwks') ensureJwksSchemas(p)
            if (step.type === 'verifyHmac') locate(p).unshift(step)
            else locate(p).push(step)
            // Publishing needs the service to own the topic: declare it, like drawing the arrow would.
            if (step.type === 'publish') declareProducer(p, ctx.service, step.topic)
          })
        }
      />
    </div>
  )
}

function AddStep(props: { allowRespond: boolean; allowFail: boolean; allowSignature: boolean; onAdd: (t: StepType) => void }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="add-step">
      <button type="button" className="add-step-btn" onClick={() => setOpen(!open)} aria-expanded={open}>
        <Icon name="plus" size={14} /> Ajouter une étape
      </button>
      {open && (
        <div className="widget-grid" role="menu">
          {STEP_ORDER.filter((t) => (t !== 'respond' || props.allowRespond) && ((t !== 'fail' && t !== 'rateLimit') || props.allowFail) && (t !== 'verifyHmac' || props.allowSignature) && (t !== 'verifyJwt' || props.allowFail)).map((t) => (
            <button
              key={t}
              type="button"
              role="menuitem"
              className={`widget ${t}`}
              onClick={() => {
                props.onAdd(t)
                setOpen(false)
              }}
            >
              <Icon name={t} size={18} />
              <strong>{STEP_LABELS[t].label}</strong>
              <small>{STEP_LABELS[t].hint}</small>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

function cloneStep(step: Step): Step {
  const copy = structuredClone(step)
  const reId = (s: Step) => {
    s.id = newId()
    if (s.type === 'branch') {
      s.then.forEach(reId)
      s.otherwise.forEach(reId)
    }
  }
  reId(copy)
  return copy
}

/** Diagnostics attached to this step itself, not to the steps nested in a branch. */
function ownDiagnostics(all: Diagnostic[], path: string): Diagnostic[] {
  return all.filter(
    (d) =>
      (d.path === path || d.path.startsWith(`${path}/`)) &&
      !d.path.startsWith(`${path}/then`) &&
      !d.path.startsWith(`${path}/otherwise`),
  )
}

const OP_SYMBOL: Record<CmpOp, string> = { eq: '=', ne: '≠', gt: '>', ge: '≥', lt: '<', le: '≤' }


function summary(step: Step): string {
  switch (step.type) {
    case 'log':
      return step.message
    case 'let':
      return `${step.name} : ${step.valueType.kind}`
    case 'publish':
      return step.topic
    case 'callRpc':
      return `${step.service}.${step.method}`
    case 'branch':
      return step.condition.kind === 'compare'
        ? `si ${exprText(step.condition.left)} ${OP_SYMBOL[step.condition.op]} ${exprText(step.condition.right)}`
        : `si ${step.condition.code}`
    case 'respond':
      return ''
    case 'fail':
      return `${step.status} ${step.message}`
    case 'rateLimit':
      return `${step.max} par ${step.windowSecs} s`
    case 'verifyHmac':
      return step.header
    case 'signJwt':
      return `→ ${step.bind}`
    case 'verifyJwt':
      return `→ ${step.bind}`
    case 'jwks':
      return `→ ${step.bind}`
    case 'hashPassword':
    case 'randomToken':
    case 'sha256':
    case 'encrypt':
    case 'decrypt':
      return `→ ${step.bind}`
    case 'verifyPassword':
      return `→ ${step.bind}`
    case 'sql':
      return `${OP_LABELS[step.op].toLowerCase()} ${step.table}${step.op === 'select' && step.bind ? ` → ${step.bind}` : ''}`
    case 'code':
      return ''
  }
}

function StepCard(props: {
  step: Step
  index: number
  count: number
  path: string
  scope: Scope
  ctx: HandlerCtx
  diagnostics: Diagnostic[]
  mutate: (fn: (list: Step[]) => void) => void
  locate: (p: Project) => Step[]
}) {
  const { step, index, mutate } = props
  const own = ownDiagnostics(props.diagnostics, props.path)
  const errors = own.filter((d) => d.severity === 'error')
  const warnings = own.filter((d) => d.severity === 'warning')
  const move = (delta: number) =>
    mutate((list) => {
      const j = index + delta
      if (j < 0 || j >= list.length) return
      const [item] = list.splice(index, 1)
      list.splice(j, 0, item!)
    })

  return (
    <Collapsible
      tone="step"
      title={
        <>
          <span className={`step-icon ${step.type}`}>
            <Icon name={step.type} size={14} />
          </span>
          <strong>{STEP_LABELS[step.type].label}</strong>
          <span className="step-summary">{summary(step)}</span>
        </>
      }
      badge={
        <>
          {errors.length > 0 && <Badge kind="error" count={errors.length} title={errors.map((d) => d.message).join('\n')} />}
          {warnings.length > 0 && <Badge kind="warning" count={warnings.length} title={warnings.map((d) => d.message).join('\n')} />}
        </>
      }
      actions={
        <span className="card-actions">
          <IconButton icon="up" title="Monter" disabled={index === 0} onClick={() => move(-1)} />
          <IconButton icon="down" title="Descendre" disabled={index === props.count - 1} onClick={() => move(1)} />
          <IconButton icon="copy" title="Dupliquer" onClick={() => mutate((list) => list.splice(index + 1, 0, cloneStep(list[index]!)))} />
          <IconButton icon="trash" title="Supprimer l’étape" danger onClick={() => mutate((list) => list.splice(index, 1))} />
        </span>
      }
    >
      {own.length > 0 && (
        <ul className="step-problems">
          {own.map((d, i) => (
            <li key={i} className={d.severity}>
              {d.message}
            </li>
          ))}
        </ul>
      )}
      <StepBody {...props} update={(fn) => mutate((list) => fn(list[index]!))} />
    </Collapsible>
  )
}

type Update = (fn: (s: Step) => void) => void

/** Narrows the step inside an update callback. */
function as<T extends StepType>(update: Update) {
  return (fn: (s: Extract<Step, { type: T }>) => void) => update((s) => fn(s as Extract<Step, { type: T }>))
}

function StepBody(props: {
  step: Step
  index: number
  path: string
  scope: Scope
  ctx: HandlerCtx
  locate: (p: Project) => Step[]
  update: Update
}) {
  const { step, scope } = props
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)

  switch (step.type) {
    case 'log': {
      const u = as<'log'>(props.update)
      const insertable = candidates(project, scope)
        .filter((c) => isSimpleScalar(c.type))
        .slice(0, 8)
      return (
        <div className="stack">
          <div className="form-row">
            <select aria-label="Niveau" value={step.level} onChange={(e) => u((s) => (s.level = e.target.value as LogLevel))}>
              <option value="debug">debug</option>
              <option value="info">info</option>
              <option value="warn">warn</option>
              <option value="error">error</option>
            </select>
            <input
              aria-label="Message"
              value={step.message}
              placeholder="commande {input.order_id} reçue"
              onChange={(e) => u((s) => (s.message = e.target.value))}
            />
          </div>
          {insertable.length > 0 && (
            <div className="chips">
              <small>Insérer :</small>
              {insertable.map((c) => (
                <button key={c.path} type="button" className="chip mono" onClick={() => u((s) => (s.message += `{${c.path}}`))}>
                  {c.path}
                </button>
              ))}
            </div>
          )}
        </div>
      )
    }

    case 'let': {
      const u = as<'let'>(props.update)
      return (
        <div className="stack">
          <div className="form-row">
            <input
              className="mono"
              aria-label="Nom de la variable"
              value={step.name}
              spellCheck={false}
              onChange={(e) => u((s) => (s.name = e.target.value))}
            />
            <select
              aria-label="Type"
              value={step.valueType.kind === 'message' ? `msg:${step.valueType.name}` : step.valueType.kind}
              onChange={(e) =>
                u((s) => {
                  if (e.target.value.startsWith('msg:')) {
                    // A whole schema: built with Rust, e.g. from queries over several tables.
                    const name = e.target.value.slice(4)
                    s.valueType = { kind: 'message', name }
                    s.value = { kind: 'raw', code: `pb::${name} { ..Default::default() }` }
                  } else {
                    s.valueType = { kind: e.target.value } as FieldType
                    s.value = { kind: 'literal', value: literalDefault(e.target.value, s.repeated) }
                  }
                })
              }
            >
              <optgroup label="Types simples">
                {SCALARS.map((k) => (
                  <option key={k}>{k}</option>
                ))}
              </optgroup>
              {project.messages.length > 0 && (
                <optgroup label="Schémas (construits en Rust)">
                  {project.messages.map((m) => (
                    <option key={m.name} value={`msg:${m.name}`}>{m.name}</option>
                  ))}
                </optgroup>
              )}
            </select>
          </div>
          <div className="form-row">
            <label className="check" title="La valeur peut être absente (Option)">
              <input
                type="checkbox"
                aria-label="Optionnel"
                checked={step.optional && !step.repeated}
                disabled={step.repeated || step.valueType.kind === 'message'}
                onChange={(e) => u((s) => (s.optional = e.target.checked))}
              />{' '}
              optionnel
            </label>
            <label className="check" title="Une liste de valeurs (Vec)">
              <input
                type="checkbox"
                aria-label="Liste"
                checked={step.repeated}
                onChange={(e) =>
                  u((s) => {
                    s.repeated = e.target.checked
                    s.optional = false
                    // A schema list stays built with Rust; for the others the old literal no longer has the right shape.
                    if (s.valueType.kind !== 'message') s.value = { kind: 'literal', value: literalDefault(s.valueType.kind, s.repeated) }
                  })
                }
              />{' '}
              liste
            </label>
          </div>
          {step.valueType.kind === 'message' ? (
            <>
              <textarea
                className="mono code-input"
                aria-label="Code Rust de la variable"
                rows={Math.min(14, Math.max(4, (step.value.kind === 'raw' ? step.value.code : '').split('\n').length + 1))}
                spellCheck={false}
                value={step.value.kind === 'raw' ? step.value.code : ''}
                onChange={(e) => u((s) => (s.value = { kind: 'raw', code: e.target.value }))}
              />
              <small className="muted">
                Expression Rust qui renvoie <code>pb::{step.valueType.name}</code> : <code>ctx</code>, <code>input</code> et <code>tx</code> sont disponibles
                (requêtes SQL sur plusieurs tables, par exemple). Ses champs se lisent ensuite comme <code>{step.name}.champ</code>.
              </small>
            </>
          ) : (
            <ExprEditor
              label="Valeur"
              expr={step.value}
              target={{ type: step.valueType, repeated: step.repeated, optional: step.optional && !step.repeated }}
              scope={scope}
              onChange={(v) => v && u((s) => (s.value = v))}
            />
          )}
        </div>
      )
    }

    case 'publish': {
      const u = as<'publish'>(props.update)
      const topic = project.topics.find((t) => t.name === step.topic)
      return (
        <div className="stack">
          <label className="form-row">
            <span>Topic</span>
            <select
              value={step.topic}
              onChange={(e) => {
                const next = e.target.value
                u((s) => {
                  s.topic = next
                  s.payload = []
                })
                edit((p) => declareProducer(p, props.ctx.service, next))
              }}
            >
              {!topic && <option value={step.topic}>{step.topic || 'Choisir…'}</option>}
              {project.topics.map((t) => (
                <option key={t.name}>{t.name}</option>
              ))}
            </select>
          </label>
          {topic && (
            <AssignmentsEditor message={topic.message} assignments={step.payload} scope={scope} update={(fn) => u((s) => fn(s.payload))} />
          )}
        </div>
      )
    }

    case 'callRpc': {
      const u = as<'callRpc'>(props.update)
      const target = project.services.find((s) => s.name === step.service)
      const rpc = target?.rpcs.find((r) => r.name === step.method)
      const callable = project.services.filter((s) => s.name !== props.ctx.service && s.rpcs.length > 0)
      return (
        <div className="stack">
          <div className="form-row">
            <select
              aria-label="Service"
              value={step.service}
              onChange={(e) =>
                u((s) => {
                  s.service = e.target.value
                  s.method = project.services.find((x) => x.name === e.target.value)?.rpcs[0]?.name ?? ''
                  s.request = []
                })
              }
            >
              {!callable.some((c) => c.name === step.service) && <option value={step.service}>{step.service || 'Service…'}</option>}
              {callable.map((c) => (
                <option key={c.name}>{c.name}</option>
              ))}
            </select>
            <select
              aria-label="Méthode"
              value={step.method}
              onChange={(e) =>
                u((s) => {
                  s.method = e.target.value
                  s.request = []
                })
              }
            >
              {!rpc && <option value={step.method}>{step.method || 'RPC…'}</option>}
              {target?.rpcs.map((r) => (
                <option key={r.name}>{r.name}</option>
              ))}
            </select>
          </div>
          {rpc && <AssignmentsEditor message={rpc.request} assignments={step.request} scope={scope} update={(fn) => u((s) => fn(s.request))} />}
          <label className="form-row">
            <span>Mémoriser la réponse dans</span>
            <input
              className="mono"
              aria-label="Variable de réponse"
              value={step.bind ?? ''}
              placeholder={rpc ? `ex. ${rpc.name.replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase()}_result` : 'variable'}
              spellCheck={false}
              onChange={(e) => u((s) => (s.bind = e.target.value === '' ? null : e.target.value))}
            />
          </label>
        </div>
      )
    }

    case 'branch': {
      const u = as<'branch'>(props.update)
      const child = (key: 'then' | 'otherwise') => (p: Project) => (props.locate(p)[props.index] as Extract<Step, { type: 'branch' }>)[key]
      return (
        <div className="stack">
          <ConditionEditor condition={step.condition} scope={scope} onChange={(c) => u((s) => (s.condition = c))} />
          <div className="branch">
            <h4>Alors</h4>
            <StepList steps={step.then} base={`${props.path}/then`} scope={scope} ctx={props.ctx} locate={child('then')} />
          </div>
          <div className="branch">
            <h4>Sinon</h4>
            <StepList steps={step.otherwise} base={`${props.path}/otherwise`} scope={scope} ctx={props.ctx} locate={child('otherwise')} />
          </div>
        </div>
      )
    }

    case 'respond': {
      const u = as<'respond'>(props.update)
      return props.ctx.response ? (
        <div className="stack">
          <AssignmentsEditor message={props.ctx.response} assignments={step.fields} scope={scope} update={(fn) => u((s) => fn(s.fields))} />
          {props.ctx.http && <RespondExtras step={step} update={u} scope={scope} />}
        </div>
      ) : (
        <Empty>« Répondre » n’a de sens que dans un RPC ou une route.</Empty>
      )
    }

    case 'signJwt':
      return <SignJwtBody step={step} update={as<'signJwt'>(props.update)} scope={scope} />
    case 'verifyJwt':
      return <VerifyJwtBody step={step} update={as<'verifyJwt'>(props.update)} scope={scope} />
    case 'jwks':
      return <JwksBody step={step} update={as<'jwks'>(props.update)} />

    case 'verifyHmac': {
      const u = as<'verifyHmac'>(props.update)
      return (
        <div className="stack">
          <div className="form-row">
            <span>En-tête</span>
            <input className="mono" aria-label="En-tête de la signature" value={step.header} spellCheck={false} onChange={(e) => u((s) => (s.header = e.target.value))} />
          </div>
          <div className="form-row">
            <span>Secret</span>
            <input className="mono" aria-label="Variable d’environnement du secret" value={step.secretEnv} spellCheck={false} onChange={(e) => u((s) => (s.secretEnv = e.target.value))} />
          </div>
          <div className="form-row">
            <span>Tolérance</span>
            <NumberInput label="Tolérance d’horloge (secondes)" min={1} value={step.toleranceSecs} onChange={(n) => u((s) => (s.toleranceSecs = n))} />
            <small className="muted">secondes</small>
          </div>
          <small className="muted">
            La signature est le HMAC-SHA256 de <code>horodatage.corps</code>, envoyée dans l’en-tête sous la forme <code>t=…,v1=…</code> (le schéma de
            Stripe). Elle est vérifiée sur le <strong>corps brut</strong>, avant qu’il soit lu comme JSON ; au-delà de la tolérance, l’horodatage est
            refusé, ce qui arrête le rejeu. Toute erreur répond <code>401 invalid signature</code>, sans dire pourquoi. Le secret est lu dans la variable
            d’environnement : le service ne démarre pas sans elle.
          </small>
        </div>
      )
    }

    case 'rateLimit': {
      const u = as<'rateLimit'>(props.update)
      return (
        <div className="stack">
          <ExprEditor label="Clé" expr={step.key} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.key = v))} />
          <div className="form-row">
            <NumberInput label="Essais maximum" min={1} value={step.max} onChange={(n) => u((s) => (s.max = n))} />
            <small className="muted">essais en</small>
            <NumberInput label="Fenêtre (secondes)" min={1} value={step.windowSecs} onChange={(n) => u((s) => (s.windowSecs = n))} />
            <small className="muted">secondes</small>
          </div>
          <small className="muted">
            Chaque essai est compté sous la clé (un e-mail, une adresse). Au-delà, la route répond <code>429</code> avec <code>Retry-After</code>.
            Fenêtre glissante, comptée en mémoire dans chaque instance du service : avec plusieurs réplicas, la limite réelle est multipliée
            d’autant.
          </small>
        </div>
      )
    }

    case 'hashPassword':
    case 'verifyPassword':
    case 'randomToken':
    case 'sha256':
    case 'encrypt':
    case 'decrypt':
      return <SecretBody step={step} update={props.update} scope={scope} />

    case 'fail': {
      const u = as<'fail'>(props.update)
      return (
        <div className="stack">
          <div className="form-row">
            <select aria-label="Statut" value={step.status} onChange={(e) => u((s) => ((s.status = Number(e.target.value)), (s.retryAfter = [429, 503].includes(s.status) ? s.retryAfter : null)))}>
              {(ERROR_STATUSES.some(([n]) => n === step.status) ? ERROR_STATUSES : [[step.status, ''] as [number, string], ...ERROR_STATUSES]).map(([n, label]) => (
                <option key={n} value={n}>{label ? `${n} · ${label}` : n}</option>
              ))}
            </select>
            <input aria-label="Message" value={step.message} placeholder="ce que le client reçoit" onChange={(e) => u((s) => (s.message = e.target.value))} />
          </div>
          {[429, 503].includes(step.status) && (
            <ExprEditor label="Réessayer après (secondes)" expr={step.retryAfter} target={scalar('int64')} scope={scope} allowUnset onChange={(v) => u((s) => (s.retryAfter = v))} />
          )}
          <small className="muted">
            Le client reçoit <code>{'{ "error": message }'}</code>. Pour un statut 5xx, le message reste dans les logs et le client reçoit un texte
            générique. Ce que la route a écrit en base et publié est annulé.
          </small>
        </div>
      )
    }

    case 'sql':
      return <SqlBody step={step} update={as<'sql'>(props.update)} scope={scope} service={props.ctx.service} http={props.ctx.http} />

    case 'code': {
      const u = as<'code'>(props.update)
      return (
        <div className="stack">
          <textarea
            className="mono code-input"
            aria-label="Code Rust"
            rows={Math.min(14, Math.max(4, step.code.split('\n').length + 1))}
            spellCheck={false}
            value={step.code}
            onChange={(e) => u((s) => (s.code = e.target.value))}
          />
          <small className="muted">
            Instructions Rust insérées telles quelles. Disponibles : <code>ctx</code>, <code>input</code>, <code>tx</code> (la transaction
            PostgreSQL du service) et les variables ci-dessus. Faites <code>tx.commit().await?;</code> avant un <code>return</code> anticipé,
            sinon la transaction et les événements publiés sont annulés.
          </small>
        </div>
      )
    }
  }
}

const TEXT = scalar('string')

const SUCCESS_STATUSES = [200, 201, 202]
const SAME_SITE: { value: SameSite; label: string }[] = [
  { value: 'lax', label: 'Lax (défaut)' },
  { value: 'strict', label: 'Strict' },
  { value: 'none', label: 'None (inter-sites)' },
]

/** What only an HTTP route can answer with: a success status of its own, and cookies. */
function RespondExtras(props: { step: Extract<Step, { type: 'respond' }>; update: (fn: (s: Extract<Step, { type: 'respond' }>) => void) => void; scope: Scope }) {
  const { step, update, scope } = props
  const newCookie = (): Cookie => ({ name: 'session', value: { kind: 'literal', value: '' }, maxAgeSecs: null, httpOnly: true, secure: true, sameSite: 'lax', path: null })
  return (
    <div className="stack">
      <div className="form-row">
        <small className="muted">Statut</small>
        <select aria-label="Statut de la réponse" value={step.status ?? ''} onChange={(e) => update((s) => (s.status = e.target.value === '' ? null : Number(e.target.value)))}>
          <option value="">celui de la route</option>
          {SUCCESS_STATUSES.map((n) => (
            <option key={n} value={n}>{n}</option>
          ))}
        </select>
      </div>
      {step.cookies.map((c, i) => (
        <div key={i} className="card" role="group" aria-label={`Cookie ${c.name}`}>
          <div className="stack">
            <div className="form-row">
              <input className="mono" aria-label="Nom du cookie" value={c.name} spellCheck={false} onChange={(e) => update((s) => (s.cookies[i]!.name = e.target.value))} />
              <IconButton icon="trash" title={`Supprimer le cookie ${c.name}`} danger onClick={() => update((s) => void s.cookies.splice(i, 1))} />
            </div>
            <ExprEditor label="Valeur du cookie" expr={c.value} target={TEXT} scope={scope} onChange={(v) => v && update((s) => (s.cookies[i]!.value = v))} />
            <div className="form-row">
              <label className="check" title="Sans durée, le cookie disparaît à la fermeture du navigateur">
                <input type="checkbox" aria-label="Durée limitée" checked={c.maxAgeSecs !== null} onChange={(e) => update((s) => (s.cookies[i]!.maxAgeSecs = e.target.checked ? 900 : null))} /> expire après
              </label>
              {c.maxAgeSecs !== null && <NumberInput label="Durée du cookie (secondes)" min={0} value={c.maxAgeSecs} onChange={(n) => update((s) => (s.cookies[i]!.maxAgeSecs = n))} />}
              {c.maxAgeSecs !== null && <small className="muted">secondes (0 supprime le cookie)</small>}
            </div>
            <div className="form-row">
              <label className="check" title="Illisible par le JavaScript de la page">
                <input type="checkbox" aria-label="HttpOnly" checked={c.httpOnly} onChange={(e) => update((s) => (s.cookies[i]!.httpOnly = e.target.checked))} /> HttpOnly
              </label>
              <label className="check" title="Envoyé en HTTPS seulement">
                <input type="checkbox" aria-label="Secure" checked={c.secure} onChange={(e) => update((s) => (s.cookies[i]!.secure = e.target.checked))} /> Secure
              </label>
              <select aria-label="SameSite" value={c.sameSite} onChange={(e) => update((s) => (s.cookies[i]!.sameSite = e.target.value as SameSite))}>
                {SAME_SITE.map((o) => (
                  <option key={o.value} value={o.value}>{o.label}</option>
                ))}
              </select>
            </div>
          </div>
        </div>
      ))}
      <button type="button" className="link-btn" onClick={() => update((s) => void s.cookies.push(newCookie()))}>
        + Ajouter un cookie
      </button>
    </div>
  )
}
/** A stored hash may be absent: an unknown account. */
const MAYBE_TEXT = { ...scalar('string'), optional: true }

const SECRET_HELP: Record<'hashPassword' | 'verifyPassword' | 'randomToken' | 'sha256' | 'encrypt' | 'decrypt', string> = {
  encrypt:
    'AES-256-GCM. Le résultat est le nonce suivi du texte chiffré, en hexadécimal ; le nonce est aléatoire, donc le même texte ne donne jamais deux fois le même résultat. La clé est de 32 octets en 64 caractères hexadécimaux, dans la variable d’environnement : le service ne démarre pas sans elle.',
  decrypt:
    'Rend le texte scellé par « Chiffrer » avec la même clé. Une clé différente, une valeur tronquée ou modifiée est une erreur interne (500) : ce qui est stocké n’a pas été écrit par ce service, ou a été altéré.',
  hashPassword:
    'argon2id, calculé hors du runtime async (il est lent exprès). Le résultat est le texte $argon2id$… à stocker ; plus de 1 024 octets est refusé (400).',
  verifyPassword:
    'Vrai si le mot de passe correspond. Sans hash enregistré (compte inconnu), la vérification tourne quand même contre un hash factice : elle prend le même temps et répond faux, donc le délai ne révèle pas quels comptes existent.',
  randomToken: '256 bits aléatoires en 64 caractères hexadécimaux. Stockez seulement son empreinte (SHA-256), jamais le jeton.',
  sha256: '64 caractères hexadécimaux. Un jeton est déjà très aléatoire : un hash rapide suffit, inutile d’employer argon2.',
}

/** The four steps that bind a variable from a secret: a password, a token or their hash. */
function SecretBody(props: {
  step: Extract<Step, { type: 'hashPassword' | 'verifyPassword' | 'randomToken' | 'sha256' | 'encrypt' | 'decrypt' }>
  update: Update
  scope: Scope
}) {
  const { step, scope } = props
  const u = (fn: (s: typeof step) => void) => props.update((s) => fn(s as typeof step))
  return (
    <div className="stack">
      {step.type === 'verifyPassword' ? (
        <>
          <ExprEditor label="Mot de passe" expr={step.password} target={TEXT} scope={scope} onChange={(v) => v && u((s) => s.type === 'verifyPassword' && (s.password = v))} />
          <ExprEditor label="Hash enregistré" expr={step.stored} target={MAYBE_TEXT} scope={scope} onChange={(v) => v && u((s) => s.type === 'verifyPassword' && (s.stored = v))} />
        </>
      ) : step.type !== 'randomToken' ? (
        <ExprEditor
          label={step.type === 'hashPassword' ? 'Mot de passe' : step.type === 'decrypt' ? 'Texte chiffré' : 'Texte'}
          expr={step.input}
          target={TEXT}
          scope={scope}
          onChange={(v) => v && u((s) => 'input' in s && (s.input = v))}
        />
      ) : null}
      {(step.type === 'encrypt' || step.type === 'decrypt') && (
        <div className="form-row">
          <span>Clé</span>
          <input className="mono" aria-label="Variable d’environnement de la clé" value={step.keyEnv} spellCheck={false} onChange={(e) => u((s) => 'keyEnv' in s && (s.keyEnv = e.target.value))} />
        </div>
      )}
      <div className="form-row">
        <input className="mono" aria-label="Nom de la variable" value={step.bind} spellCheck={false} onChange={(e) => u((s) => (s.bind = e.target.value))} />
        <small className="muted">reçoit {step.type === 'verifyPassword' ? 'vrai ou faux' : 'le texte'}</small>
      </div>
      <small className="muted">{SECRET_HELP[step.type]}</small>
    </div>
  )
}

const OPS: { value: CmpOp; label: string }[] = [
  { value: 'eq', label: '=' },
  { value: 'ne', label: '≠' },
  { value: 'gt', label: '>' },
  { value: 'ge', label: '≥' },
  { value: 'lt', label: '<' },
  { value: 'le', label: '≤' },
]

function ConditionEditor(props: { condition: Condition; scope: Scope; onChange: (c: Condition) => void }) {
  const project = useStore((s) => s.project)
  const c = props.condition
  const leftType =
    c.kind === 'compare' && c.left.kind === 'path'
      ? (candidates(project, props.scope).find((x) => x.path === (c.left as { path: string }).path)?.type ?? null)
      : null

  return (
    <div className="stack condition">
      <div className="form-row">
        <span>Si</span>
        <select
          aria-label="Type de condition"
          value={c.kind}
          onChange={(e) =>
            props.onChange(
              e.target.value === 'raw'
                ? { kind: 'raw', code: 'true' }
                : { kind: 'compare', left: { kind: 'raw', code: 'true' }, op: 'eq', right: { kind: 'literal', value: true } },
            )
          }
        >
          <option value="compare">comparaison</option>
          <option value="raw">expression Rust</option>
        </select>
      </div>
      {c.kind === 'raw' ? (
        <input
          className="mono"
          aria-label="Condition Rust"
          value={c.code}
          spellCheck={false}
          onChange={(e) => props.onChange({ kind: 'raw', code: e.target.value })}
        />
      ) : (
        <>
          <ExprEditor label="Gauche" expr={c.left} target={null} scope={props.scope} onChange={(v) => v && props.onChange({ ...c, left: v })} />
          <select
            aria-label="Opérateur"
            className="op"
            value={c.op}
            onChange={(e) => props.onChange({ ...c, op: e.target.value as CmpOp })}
          >
            {OPS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
          <ExprEditor label="Droite" expr={c.right} target={leftType} scope={props.scope} onChange={(v) => v && props.onChange({ ...c, right: v })} />
        </>
      )}
    </div>
  )
}
