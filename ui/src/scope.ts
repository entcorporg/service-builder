// Client-side mirror of the type rules of crates/sb-model/src/typing.rs. It only drives
// suggestions (which variables fit a field); the Rust validator stays authoritative.
import type { Column, ColumnType, Field, FieldType, Message, Project, Service, Step, Table } from './types'

export interface TypeRef {
  type: FieldType
  repeated: boolean
  optional: boolean
}

/** Variables visible at a point of a handler, mapped to their type. */
export type Scope = Record<string, TypeRef>

export const SCALARS = ['string', 'uuid', 'bool', 'int32', 'int64', 'float', 'double'] as const

export function isMessage(t: FieldType): t is { kind: 'message'; name: string } {
  return t.kind === 'message'
}

export function typeOfField(f: Field): TypeRef {
  return {
    type: f.type,
    repeated: f.repeated,
    optional: f.optional && !isMessage(f.type) && !f.repeated,
  }
}

export function scalar(kind: FieldType['kind']): TypeRef {
  return { type: { kind } as FieldType, repeated: false, optional: false }
}

const rawName = (t: FieldType): string => (isMessage(t) ? t.name : t.kind)

export function fieldTypeLabel(t: FieldType): string {
  // The derived row of a SQL step: `OrdersRowH6` reads as "ligne orders".
  const row = isMessage(t) ? /^(.+)Row[A-Z0-9][A-Za-z0-9]*$/.exec(t.name) : null
  return isMessage(t) ? (row ? `ligne ${row[1]!.toLowerCase()}` : t.name) : t.kind
}

export function typeLabel(t: TypeRef): string {
  return `${t.repeated ? 'liste de ' : ''}${t.optional ? 'optionnel ' : ''}${fieldTypeLabel(t.type)}`
}

/** Mirrors `FieldType::assignable_from`: the same type, or `uuid` and `string`, which are both text. */
function sameBase(a: FieldType, b: FieldType): boolean {
  const text = (t: FieldType) => t.kind === 'string' || t.kind === 'uuid'
  return (rawName(a) === rawName(b) && isMessage(a) === isMessage(b)) || (text(a) && text(b))
}

export function sameType(a: TypeRef, b: TypeRef): boolean {
  return a.repeated === b.repeated && a.optional === b.optional && sameBase(a.type, b.type)
}

/** Plain scalar usable in comparisons and `Let`. */
export function isSimpleScalar(t: TypeRef): boolean {
  return !t.repeated && !t.optional && !['bytes', 'timestamp', 'message'].includes(t.type.kind)
}

/** Mirrors ResolvedPath::adapt_to. */
export function fits(from: TypeRef, bare: boolean, to: TypeRef): boolean {
  // Only a whole message variable is bare: a scalar variable (`let`, a token) widens like any field.
  if (bare && isMessage(from.type) && !from.repeated) return sameType(from, to)
  if (sameType(from, to)) return true
  return (
    !from.optional &&
    !from.repeated &&
    to.optional &&
    !to.repeated &&
    !isMessage(to.type) &&
    sameBase(from.type, to.type)
  )
}

// ---- SQL rows ----------------------------------------------------------------------------
// The row a `select` reads is typed after the table and the ticked columns: it is what makes the
// SQL block predictive. Mirrors `sql_row_name` and `Project::with_sql_rows` of sb-model.

const pascal = (s: string) =>
  s
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join('')

export function sqlRowName(table: string, stepId: string): string {
  return `${pascal(table)}Row${pascal(stepId)}`
}

/** What a flow sees of a column: uuids are uuids, timestamps and JSON documents are text. */
export function columnFieldType(t: ColumnType): FieldType {
  return { kind: t === 'timestamp' || t === 'jsonb' ? 'string' : t } as FieldType
}

export function columnType(c: Column): TypeRef {
  return { type: columnFieldType(c.type), repeated: false, optional: c.nullable }
}

const rowsCache = new WeakMap<Project, Map<string, Message>>()

/** The rows of the `select` steps of the project, by derived message name. */
export function sqlRows(project: Project): Map<string, Message> {
  const cached = rowsCache.get(project)
  if (cached) return cached
  const rows = new Map<string, Message>()
  const visit = (svc: Service, steps: Step[]) => {
    for (const s of steps) {
      if (s.type === 'branch') {
        visit(svc, s.then)
        visit(svc, s.otherwise)
      } else if (s.type === 'sql' && s.columns.length > 0 && !s.into) {
        const table = svc.tables.find((t) => t.name === s.table)
        if (!table) continue
        rows.set(sqlRowName(s.table, s.id), {
          name: sqlRowName(s.table, s.id),
          kind: 'proto',
          fields: s.columns
            .map((c) => table.columns.find((x) => x.name === c))
            .filter((c): c is Column => !!c)
            .map((c) => ({ name: c.name, type: columnFieldType(c.type), repeated: false, optional: c.nullable })),
        })
      }
    }
  }
  for (const svc of project.services) {
    visit(svc, svc.rpcs.flatMap((r) => r.steps))
    visit(svc, svc.routes.flatMap((r) => r.steps))
    visit(svc, svc.consumers.flatMap((c) => c.steps))
    visit(svc, svc.jobs.flatMap((j) => j.steps))
  }
  rowsCache.set(project, rows)
  return rows
}

/** A message of the project, or the row of a SQL step. */
export function findMessage(project: Project, name: string): Message | undefined {
  return project.messages.find((m) => m.name === name) ?? sqlRows(project).get(name)
}

export const tableOf = (service: Service | undefined, name: string): Table | undefined => service?.tables.find((t) => t.name === name)

export interface Candidate {
  path: string
  type: TypeRef
  /** The whole variable rather than one of its fields. */
  bare: boolean
}

/** Every readable value in scope: variables and (nested) message fields. */
export function candidates(project: Project, scope: Scope, maxDepth = 3): Candidate[] {
  const out: Candidate[] = []
  const walk = (prefix: string, type: TypeRef, depth: number, bare: boolean) => {
    out.push({ path: prefix, type, bare })
    if (depth >= maxDepth || type.repeated || !isMessage(type.type)) return
    const message = findMessage(project, (type.type as { name: string }).name)
    for (const f of message?.fields ?? []) walk(`${prefix}.${f.name}`, typeOfField(f), depth + 1, false)
  }
  const names = Object.keys(scope).sort((a, b) => (a === 'input' ? -1 : b === 'input' ? 1 : a.localeCompare(b)))
  for (const name of names) walk(name, scope[name]!, 0, true)
  return out
}

export function inputScope(messageName: string): Scope {
  return { input: { type: { kind: 'message', name: messageName }, repeated: false, optional: false } }
}

/** Scope after `step` ran (variables bound by Let, CallRpc, SQL selects and the password and token steps). */
export function scopeAfter(project: Project, scope: Scope, step: Step): Scope {
  if (step.type === 'let' && step.name) {
    return { ...scope, [step.name]: { type: step.valueType, repeated: step.repeated, optional: step.optional && !step.repeated } }
  }
  if ((step.type === 'hashPassword' || step.type === 'randomToken' || step.type === 'sha256' || step.type === 'verifyPassword' || step.type === 'encrypt' || step.type === 'decrypt') && step.bind) {
    return { ...scope, [step.bind]: scalar(step.type === 'verifyPassword' ? 'bool' : 'string') }
  }
  // A select, or a step that gives back the rows it wrote (`RETURNING`).
  if (step.type === 'signJwt' && step.bind) return { ...scope, [step.bind]: scalar('string') }
  if (step.type === 'verifyJwt' && step.bind) return { ...scope, [step.bind]: { type: { kind: 'message', name: step.schema }, repeated: false, optional: false } }
  if (step.type === 'totpGenerate' && step.bind) return { ...scope, [step.bind]: { type: { kind: 'message', name: 'TotpSetup' }, repeated: false, optional: false } }
  if (step.type === 'recoveryCodes' && step.bind) return { ...scope, [step.bind]: { type: { kind: 'string' }, repeated: true, optional: false } }
  if ((step.type === 'passkeyRegisterStart' || step.type === 'passkeyLoginStart') && step.bind) return { ...scope, [step.bind]: { type: { kind: 'message', name: 'PasskeyChallenge' }, repeated: false, optional: false } }
  if (step.type === 'passkeyLoginFinish' && step.bind) return { ...scope, [step.bind]: scalar('string') }
  if (step.type === 'imageSniff' && step.bind) return { ...scope, [step.bind]: scalar('string') }
  if (step.type === 'objectGet' && step.bind) return { ...scope, [step.bind]: { type: { kind: 'string' }, repeated: false, optional: true } }
  if (step.type === 'jwks' && step.bind) return { ...scope, [step.bind]: { type: { kind: 'message', name: 'Jwks' }, repeated: false, optional: false } }
  if (step.type === 'sql' && step.columns.length > 0 && step.bind) {
    const message = step.into ?? sqlRowName(step.table, step.id)
    return { ...scope, [step.bind]: { type: { kind: 'message', name: message }, repeated: step.many, optional: false } }
  }
  if (step.type === 'callRpc' && step.bind) {
    const rpc = project.services.find((s) => s.name === step.service)?.rpcs.find((r) => r.name === step.method)
    if (rpc) {
      return { ...scope, [step.bind]: { type: { kind: 'message', name: rpc.response }, repeated: false, optional: false } }
    }
  }
  return scope
}

/** For each step of a block, the scope it runs in. */
export function scopesOf(project: Project, initial: Scope, steps: Step[]): Scope[] {
  const scopes: Scope[] = []
  let current = initial
  for (const step of steps) {
    scopes.push(current)
    current = scopeAfter(project, current, step)
  }
  return scopes
}
