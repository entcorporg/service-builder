// Client-side mirror of the type rules of crates/sb-model/src/typing.rs. It only drives
// suggestions (which variables fit a field); the Rust validator stays authoritative.
import type { Field, FieldType, Project, Step } from './types'

export interface TypeRef {
  type: FieldType
  repeated: boolean
  optional: boolean
}

/** Variables visible at a point of a handler, mapped to their type. */
export type Scope = Record<string, TypeRef>

export const SCALARS = ['string', 'bool', 'int32', 'int64', 'float', 'double'] as const

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

export function fieldTypeLabel(t: FieldType): string {
  return isMessage(t) ? t.name : t.kind
}

export function typeLabel(t: TypeRef): string {
  return `${t.repeated ? 'liste de ' : ''}${t.optional ? 'optionnel ' : ''}${fieldTypeLabel(t.type)}`
}

export function sameType(a: TypeRef, b: TypeRef): boolean {
  return (
    a.repeated === b.repeated &&
    a.optional === b.optional &&
    fieldTypeLabel(a.type) === fieldTypeLabel(b.type) &&
    isMessage(a.type) === isMessage(b.type)
  )
}

/** Plain scalar usable in comparisons and `Let`. */
export function isSimpleScalar(t: TypeRef): boolean {
  return !t.repeated && !t.optional && !['bytes', 'timestamp', 'message'].includes(t.type.kind)
}

/** Mirrors ResolvedPath::adapt_to. */
export function fits(from: TypeRef, bare: boolean, to: TypeRef): boolean {
  if (bare) return sameType(from, to)
  if (sameType(from, to)) return true
  return (
    !from.optional &&
    !from.repeated &&
    to.optional &&
    !to.repeated &&
    !isMessage(to.type) &&
    fieldTypeLabel(from.type) === fieldTypeLabel(to.type)
  )
}

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
    const message = project.messages.find((m) => m.name === (type.type as { name: string }).name)
    for (const f of message?.fields ?? []) walk(`${prefix}.${f.name}`, typeOfField(f), depth + 1, false)
  }
  const names = Object.keys(scope).sort((a, b) => (a === 'input' ? -1 : b === 'input' ? 1 : a.localeCompare(b)))
  for (const name of names) walk(name, scope[name]!, 0, true)
  return out
}

export function inputScope(messageName: string): Scope {
  return { input: { type: { kind: 'message', name: messageName }, repeated: false, optional: false } }
}

/** Scope after `step` ran (variables bound by Let and CallRpc). */
export function scopeAfter(project: Project, scope: Scope, step: Step): Scope {
  if (step.type === 'let' && step.name) {
    return { ...scope, [step.name]: { type: step.valueType, repeated: false, optional: false } }
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
