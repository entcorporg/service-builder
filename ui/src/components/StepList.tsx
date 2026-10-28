import { useState } from 'react'
import { makeStep, newId, STEP_LABELS } from '../model'
import { candidates, isSimpleScalar, SCALARS, type Scope, scopeAfter, scopesOf, scalar } from '../scope'
import { useStore } from '../store'
import type { CmpOp, Condition, Diagnostic, Expr, FieldType, LogLevel, Project, Step, StepType } from '../types'
import { AssignmentsEditor } from './AssignmentsEditor'
import { ExprEditor } from './ExprEditor'
import { Icon } from './Icon'
import { Badge, Collapsible, Empty, IconButton } from './ui'

export interface HandlerCtx {
  service: string
  /** Response message when the handler is an RPC, null for consumers. */
  response: string | null
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

const STEP_ORDER: StepType[] = ['publish', 'callRpc', 'branch', 'let', 'log', 'respond', 'code']

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
        onAdd={(type) => mutate((list) => list.push(makeStep(type, project, endScope, ctx)))}
      />
    </div>
  )
}

function AddStep(props: { allowRespond: boolean; onAdd: (t: StepType) => void }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="add-step">
      <button type="button" className="add-step-btn" onClick={() => setOpen(!open)} aria-expanded={open}>
        <Icon name="plus" size={14} /> Ajouter une étape
      </button>
      {open && (
        <div className="widget-grid" role="menu">
          {STEP_ORDER.filter((t) => t !== 'respond' || props.allowRespond).map((t) => (
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

function exprText(e: Expr): string {
  return e.kind === 'path' ? e.path : e.kind === 'raw' ? e.code : JSON.stringify(e.value)
}

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

  switch (step.type) {
    case 'log': {
      const u = as<'log'>(props.update)
      const insertable = candidates(project, scope)
        .filter((c) => isSimpleScalar(c.type))
        .slice(0, 8)
      return (
        <div className="stack">
          <div className="inline">
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
          <div className="inline">
            <input
              className="mono"
              aria-label="Nom de la variable"
              value={step.name}
              spellCheck={false}
              onChange={(e) => u((s) => (s.name = e.target.value))}
            />
            <select
              aria-label="Type"
              value={step.valueType.kind}
              onChange={(e) =>
                u((s) => {
                  s.valueType = { kind: e.target.value } as FieldType
                  s.value = { kind: 'literal', value: e.target.value === 'bool' ? false : e.target.value === 'string' ? '' : 0 }
                })
              }
            >
              {SCALARS.map((k) => (
                <option key={k}>{k}</option>
              ))}
            </select>
          </div>
          <ExprEditor
            label="Valeur"
            expr={step.value}
            target={scalar(step.valueType.kind)}
            scope={scope}
            onChange={(v) => v && u((s) => (s.value = v))}
          />
        </div>
      )
    }

    case 'publish': {
      const u = as<'publish'>(props.update)
      const topic = project.topics.find((t) => t.name === step.topic)
      return (
        <div className="stack">
          <label className="inline">
            <span>Topic</span>
            <select
              value={step.topic}
              onChange={(e) =>
                u((s) => {
                  s.topic = e.target.value
                  s.payload = []
                })
              }
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
          <div className="inline">
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
          <label className="inline">
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
        <AssignmentsEditor message={props.ctx.response} assignments={step.fields} scope={scope} update={(fn) => u((s) => fn(s.fields))} />
      ) : (
        <Empty>« Répondre » n’a de sens que dans un RPC.</Empty>
      )
    }

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
            Instructions Rust insérées telles quelles. Disponibles : <code>ctx</code>, <code>input</code> et les variables ci-dessus.
          </small>
        </div>
      )
    }
  }
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
      <div className="inline">
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
