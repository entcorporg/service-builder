import { useEffect, useMemo, useState } from 'react'
import { type Candidate, candidates, fits, isMessage, isSimpleScalar, type Scope, type TypeRef, typeLabel } from '../scope'
import type { Expr } from '../types'
import { useStore } from '../store'

type Mode = 'unset' | 'path' | 'literal' | 'raw' | 'now'

const modeOf = (e: Expr | null): Mode => (e === null ? 'unset' : e.kind)

/** What a literal can be typed as, given the field it fills. */
function literalKind(target: TypeRef | null): 'string' | 'bool' | 'int' | 'float' | 'free' | 'list' | 'none' {
  if (!target) return 'free'
  if (isMessage(target.type)) return 'none'
  if (target.repeated) return ['bytes', 'timestamp'].includes(target.type.kind) ? 'none' : 'list'
  switch (target.type.kind) {
    case 'string':
    case 'uuid':
      return 'string'
    case 'bool':
      return 'bool'
    case 'int32':
    case 'int64':
      return 'int'
    case 'float':
    case 'double':
      return 'float'
    default:
      return 'none'
  }
}

function defaultFor(mode: Mode, target: TypeRef | null, options: Candidate[]): Expr | null {
  switch (mode) {
    case 'unset':
      return null
    case 'path':
      return { kind: 'path', path: options[0]?.path ?? '' }
    case 'raw':
      return { kind: 'raw', code: '' }
    case 'now':
      return { kind: 'now' }
    case 'literal': {
      const k = literalKind(target)
      return { kind: 'literal', value: k === 'list' ? [] : k === 'bool' ? false : k === 'int' || k === 'float' ? 0 : '' }
    }
  }
}

/** Free-form literal: numbers and booleans are recognised, everything else is a string. */
function parseFree(text: string): string | number | boolean {
  if (text === 'true') return true
  if (text === 'false') return false
  if (text.trim() !== '' && Number.isFinite(Number(text))) return Number(text)
  return text
}

export function ExprEditor(props: {
  expr: Expr | null
  /** Type the expression must produce; null when unconstrained (comparisons). */
  target: TypeRef | null
  scope: Scope
  allowUnset?: boolean
  /** Offers the database clock: the value of a timestamp column of an SQL step. */
  allowNow?: boolean
  onChange: (expr: Expr | null) => void
  label: string
}) {
  const project = useStore((s) => s.project)
  const { expr, target, scope } = props

  const options = useMemo(() => {
    const all = candidates(project, scope)
    return target
      ? all.filter((c) => fits(c.type, c.bare, target))
      : all.filter((c) => isSimpleScalar(c.type))
  }, [project, scope, target])

  const mode = modeOf(expr)
  const groups = new Map<string, Candidate[]>()
  for (const c of options) {
    const root = c.path.split('.')[0]!
    groups.set(root, [...(groups.get(root) ?? []), c])
  }
  const currentPath = expr?.kind === 'path' ? expr.path : null
  const known = currentPath === null || options.some((o) => o.path === currentPath)

  return (
    <div className="expr">
      <select
        className="expr-mode"
        aria-label={`${props.label} : source`}
        value={mode}
        onChange={(e) => props.onChange(defaultFor(e.target.value as Mode, target, options))}
      >
        {props.allowUnset && <option value="unset">—</option>}
        <option value="path">Champ</option>
        <option value="literal">Valeur</option>
        <option value="raw">Rust</option>
        {(props.allowNow || mode === 'now') && <option value="now">Maintenant</option>}
      </select>

      {mode === 'now' && <span className="expr-value muted" aria-label={`${props.label} : horloge`}>l’heure de la base (<code>now()</code>)</span>}

      {mode === 'path' && (
        <select
          className="expr-value"
          aria-label={`${props.label} : champ`}
          value={currentPath ?? ''}
          onChange={(e) => props.onChange({ kind: 'path', path: e.target.value })}
        >
          {!known && <option value={currentPath ?? ''}>{currentPath} (incompatible)</option>}
          {currentPath === '' && <option value="">Choisir…</option>}
          {[...groups.entries()].map(([root, list]) => (
            <optgroup key={root} label={root}>
              {list.map((c) => (
                <option key={c.path} value={c.path}>
                  {c.path} · {typeLabel(c.type)}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
      )}

      {mode === 'literal' && expr?.kind === 'literal' && <LiteralInput expr={expr} target={target} label={props.label} onChange={props.onChange} />}

      {mode === 'raw' && expr?.kind === 'raw' && (
        <input
          className="expr-value mono"
          aria-label={`${props.label} : code Rust`}
          value={expr.code}
          placeholder="expression Rust"
          spellCheck={false}
          onChange={(e) => props.onChange({ kind: 'raw', code: e.target.value })}
        />
      )}
    </div>
  )
}

function LiteralInput(props: {
  expr: Extract<Expr, { kind: 'literal' }>
  target: TypeRef | null
  label: string
  onChange: (e: Expr) => void
}) {
  const kind = literalKind(props.target)
  const { value } = props.expr
  const set = (v: string | number | boolean | null | unknown[]) => props.onChange({ kind: 'literal', value: v })
  const aria = `${props.label} : valeur`

  if (kind === 'none') {
    return <span className="expr-value muted">Type non éditable ici, utilisez « Rust »</span>
  }
  if (kind === 'list') return <ListInput value={value} label={aria} onChange={set} />
  // An optional value can be left out altogether: the literal is then `null`.
  if (props.target?.optional) {
    const present = { ...props.target, optional: false }
    const absent = value === null
    const initial = defaultFor('literal', present, [])
    return (
      <>
        {absent ? <span className="expr-value muted">aucune valeur</span> : <LiteralInput {...props} target={present} />}
        <label className="check" title="Ne pas donner de valeur">
          <input
            type="checkbox"
            aria-label={`${props.label} : absent`}
            checked={absent}
            onChange={(e) => props.onChange(e.target.checked ? { kind: 'literal', value: null } : initial!)}
          />{' '}
          absent
        </label>
      </>
    )
  }
  if (kind === 'bool') {
    return (
      <select className="expr-value" aria-label={aria} value={String(value === true)} onChange={(e) => set(e.target.value === 'true')}>
        <option value="true">vrai</option>
        <option value="false">faux</option>
      </select>
    )
  }
  if (kind === 'int' || kind === 'float') {
    return (
      <input
        className="expr-value"
        type="number"
        aria-label={aria}
        step={kind === 'int' ? 1 : 'any'}
        value={typeof value === 'number' ? value : 0}
        onChange={(e) => set(e.target.value === '' ? 0 : Number(e.target.value))}
      />
    )
  }
  return (
    <input
      className="expr-value"
      aria-label={aria}
      value={String(value)}
      placeholder={kind === 'free' ? 'texte, nombre ou true/false' : 'texte'}
      onChange={(e) => set(kind === 'free' ? parseFree(e.target.value) : e.target.value)}
    />
  )
}

/** A list is written as a JSON array; the text is only applied once it parses as one. */
function ListInput(props: { value: unknown; label: string; onChange: (v: unknown[]) => void }) {
  const text = JSON.stringify(Array.isArray(props.value) ? props.value : [])
  const [draft, setDraft] = useState(text)
  useEffect(() => setDraft(text), [text])
  const valid = (() => {
    try {
      return Array.isArray(JSON.parse(draft))
    } catch {
      return false
    }
  })()
  return (
    <input
      className="expr-value mono"
      aria-label={props.label}
      aria-invalid={!valid}
      value={draft}
      spellCheck={false}
      placeholder='["a", "b"]'
      onChange={(e) => {
        setDraft(e.target.value)
        try {
          const parsed = JSON.parse(e.target.value)
          if (Array.isArray(parsed)) props.onChange(parsed)
        } catch {
          // Keep typing: the last valid list stays applied.
        }
      }}
    />
  )
}
