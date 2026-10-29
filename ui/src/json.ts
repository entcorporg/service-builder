// Example JSON of an API schema, for the endpoint editor: what a front sends or receives.
import type { Field, Project } from './types'

const SCALAR_EXAMPLE: Record<string, unknown> = {
  string: 'string',
  bool: false,
  int32: 0,
  int64: 0,
  float: 0,
  double: 0,
  bytes: '',
  timestamp: '1970-01-01T00:00:00Z',
}

function fieldExample(project: Project, f: Field, depth: number): unknown {
  const one =
    f.type.kind === 'message'
      ? depth <= 0
        ? {}
        : objectExample(project, f.type.name, depth - 1)
      : (SCALAR_EXAMPLE[f.type.kind] ?? null)
  return f.repeated ? [one] : one
}

function objectExample(project: Project, message: string, depth: number, include: (f: Field) => boolean = () => true) {
  const out: Record<string, unknown> = {}
  const msg = project.messages.find((m) => m.name === message)
  for (const f of msg?.fields ?? []) if (include(f)) out[f.name] = fieldExample(project, f, depth)
  return out
}

/** Pretty JSON of `message` with placeholder values; `include` keeps only some fields (the body, without the path parameters). */
export function exampleJson(project: Project, message: string, include?: (f: Field) => boolean): string {
  return JSON.stringify(objectExample(project, message, 3, include), null, 2)
}

/** `?limit=0&q=string` for the fields a GET or DELETE reads from the query string. */
export function exampleQuery(project: Project, message: string, include: (f: Field) => boolean): string {
  const msg = project.messages.find((m) => m.name === message)
  const parts = (msg?.fields ?? []).filter(include).map((f) => `${f.name}=${String(SCALAR_EXAMPLE[f.type.kind] ?? '')}`)
  return parts.length ? `?${parts.join('&')}` : ''
}
