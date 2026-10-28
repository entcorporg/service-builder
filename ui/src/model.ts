// Editing helpers on the project document: factories and rename cascades.
// All mutate a draft (immer), never the live state.
import { candidates, fits, type Scope, typeOfField } from './scope'
import type { Assignment, Message, Project, Service, Step, StepType } from './types'

export function newId(): string {
  return crypto.randomUUID().slice(0, 8)
}

export function uniqueName(base: string, taken: Iterable<string>): string {
  const set = new Set(taken)
  if (!set.has(base)) return base
  for (let i = 2; ; i++) if (!set.has(`${base}${i}`)) return `${base}${i}`
}

// ---- walking ----------------------------------------------------------------------

export function walkSteps(steps: Step[], fn: (s: Step) => void) {
  for (const s of steps) {
    fn(s)
    if (s.type === 'branch') {
      walkSteps(s.then, fn)
      walkSteps(s.otherwise, fn)
    }
  }
}

function allHandlers(p: Project) {
  return p.services.flatMap((s) => [
    ...s.rpcs.map((r) => ({ steps: r.steps, response: r.response as string | null })),
    ...s.consumers.map((c) => ({ steps: c.steps, response: null as string | null })),
  ])
}

/** Services that publish to or consume `topic`. */
export function servicesUsingTopic(p: Project, topic: string): Service[] {
  return p.services.filter((s) => {
    if (s.consumers.some((c) => c.topic === topic)) return true
    let publishes = false
    for (const steps of [...s.rpcs.map((r) => r.steps), ...s.consumers.map((c) => c.steps)])
      walkSteps(steps, (st) => {
        if (st.type === 'publish' && st.topic === topic) publishes = true
      })
    return publishes
  })
}

// ---- rename cascades --------------------------------------------------------------

export function renameMessage(p: Project, from: string, to: string) {
  for (const m of p.messages) {
    if (m.name === from) m.name = to
    for (const f of m.fields) if (f.type.kind === 'message' && f.type.name === from) f.type.name = to
  }
  for (const t of p.topics) if (t.message === from) t.message = to
  for (const s of p.services)
    for (const r of s.rpcs) {
      if (r.request === from) r.request = to
      if (r.response === from) r.response = to
    }
}

export function renameTopic(p: Project, from: string, to: string) {
  for (const t of p.topics) if (t.name === from) t.name = to
  for (const s of p.services) for (const c of s.consumers) if (c.topic === from) c.topic = to
  for (const h of allHandlers(p))
    walkSteps(h.steps, (st) => {
      if (st.type === 'publish' && st.topic === from) st.topic = to
    })
}

export function renameService(p: Project, from: string, to: string) {
  for (const s of p.services) if (s.name === from) s.name = to
  for (const h of allHandlers(p))
    walkSteps(h.steps, (st) => {
      if (st.type === 'callRpc' && st.service === from) st.service = to
    })
}

export function renameRpc(p: Project, service: string, from: string, to: string) {
  for (const h of allHandlers(p))
    walkSteps(h.steps, (st) => {
      if (st.type === 'callRpc' && st.service === service && st.method === from) st.method = to
    })
  const rpc = p.services.find((s) => s.name === service)?.rpcs.find((r) => r.name === from)
  if (rpc) rpc.name = to
}

/** Renames a field and the mappings that target it. Paths reading it are left to the validator. */
export function renameField(p: Project, message: string, from: string, to: string) {
  const msg = p.messages.find((m) => m.name === message)
  const field = msg?.fields.find((f) => f.name === from)
  if (!field) return
  field.name = to
  const fix = (list: Assignment[]) => {
    for (const a of list) if (a.field === from) a.field = to
  }
  for (const t of p.topics) if (t.message === message && t.keyField === from) t.keyField = to
  for (const s of p.services) {
    const visit = (steps: Step[], response: string | null) =>
      walkSteps(steps, (st) => {
        if (st.type === 'publish' && p.topics.find((t) => t.name === st.topic)?.message === message) fix(st.payload)
        if (st.type === 'callRpc') {
          const rpc = p.services.find((x) => x.name === st.service)?.rpcs.find((r) => r.name === st.method)
          if (rpc?.request === message) fix(st.request)
        }
        if (st.type === 'respond' && response === message) fix(st.fields)
      })
    for (const r of s.rpcs) visit(r.steps, r.response)
    for (const c of s.consumers) visit(c.steps, null)
  }
}

// ---- factories --------------------------------------------------------------------

export function addMessage(p: Project, base = 'NewMessage'): Message {
  const msg: Message = {
    name: uniqueName(base, p.messages.map((m) => m.name)),
    fields: [{ name: 'id', type: { kind: 'string' }, repeated: false, optional: false, tag: 1 }],
  }
  p.messages.push(msg)
  return msg
}

/** Protobuf numbers are never reused: take one past the highest in use. */
export function nextTag(m: Message): number {
  return Math.max(0, ...m.fields.map((f, i) => f.tag ?? i + 1)) + 1
}

export function addService(p: Project, position: { x: number; y: number }): Service {
  const port = Math.max(50050, ...p.services.map((s) => s.port)) + 1
  const svc: Service = {
    name: uniqueName('NewService', p.services.map((s) => s.name)),
    port,
    rpcs: [],
    consumers: [],
    position,
  }
  p.services.push(svc)
  return svc
}

export function addTopic(p: Project, position: { x: number; y: number }) {
  if (p.messages.length === 0) addMessage(p, 'Event')
  const topic = {
    name: uniqueName('new.topic', p.topics.map((t) => t.name)),
    message: p.messages[0]!.name,
    partitions: 3,
    replicationFactor: 1,
    keyField: null as string | null,
    position,
  }
  p.topics.push(topic)
  return topic
}

/** New RPC with its own request/response messages. */
export function addRpc(p: Project, service: Service) {
  const name = uniqueName('NewRpc', service.rpcs.map((r) => r.name))
  const request = addMessage(p, `${name}Request`)
  const response = addMessage(p, `${name}Response`)
  const rpc = { name, request: request.name, response: response.name, steps: [] as Step[] }
  service.rpcs.push(rpc)
  return rpc
}

export function addConsumer(p: Project, service: Service) {
  const topic = p.topics[0]
  if (!topic) return null
  const consumer = {
    name: uniqueName('on_event', service.consumers.map((c) => c.name)),
    topic: topic.name,
    groupId: null as string | null,
    steps: [] as Step[],
  }
  service.consumers.push(consumer)
  return consumer
}

/** Maps every field of `message` that has a same-named, compatible value in scope. */
export function autoMap(p: Project, scope: Scope, message: string): Assignment[] {
  const msg = p.messages.find((m) => m.name === message)
  if (!msg) return []
  const cands = candidates(p, scope)
  const out: Assignment[] = []
  for (const f of msg.fields) {
    const to = typeOfField(f)
    const match = cands.find((c) => c.path.split('.').at(-1) === f.name && c.path !== 'input' && fits(c.type, c.bare, to))
    if (match) out.push({ field: f.name, value: { kind: 'path', path: match.path } })
  }
  return out
}

export const STEP_LABELS: Record<StepType, { label: string; hint: string }> = {
  log: { label: 'Journal', hint: 'Écrire une ligne de log' },
  let: { label: 'Variable', hint: 'Calculer et nommer une valeur' },
  publish: { label: 'Publier un événement', hint: 'Envoyer un message sur un topic Kafka' },
  callRpc: { label: 'Appeler un RPC', hint: 'Appeler un autre service en gRPC' },
  branch: { label: 'Condition', hint: 'Si … alors … sinon …' },
  respond: { label: 'Répondre', hint: 'Renvoyer la réponse du RPC' },
  code: { label: 'Code Rust', hint: 'Insérer du Rust écrit à la main' },
}

/** A new step with sensible defaults for the place it is inserted at. */
export function makeStep(type: StepType, p: Project, scope: Scope, ctx: { service: string; response: string | null }): Step {
  const id = newId()
  switch (type) {
    case 'log':
      return { type, id, level: 'info', message: '' }
    case 'let':
      return {
        type,
        id,
        name: uniqueName('value', Object.keys(scope)),
        valueType: { kind: 'string' },
        value: { kind: 'literal', value: '' },
      }
    case 'publish': {
      const topic = p.topics[0]
      return { type, id, topic: topic?.name ?? '', payload: topic ? autoMap(p, scope, topic.message) : [] }
    }
    case 'callRpc': {
      const target = p.services.find((s) => s.name !== ctx.service && s.rpcs.length > 0)
      const rpc = target?.rpcs[0]
      return {
        type,
        id,
        service: target?.name ?? '',
        method: rpc?.name ?? '',
        request: rpc ? autoMap(p, scope, rpc.request) : [],
        bind: null,
      }
    }
    case 'branch': {
      const left = candidates(p, scope).find((c) => c.type.type.kind !== 'message' && !c.type.repeated && !c.type.optional)
      return {
        type,
        id,
        condition: {
          kind: 'compare',
          left: left ? { kind: 'path', path: left.path } : { kind: 'raw', code: 'true' },
          op: 'eq',
          right: { kind: 'literal', value: '' },
        },
        then: [],
        otherwise: [],
      }
    }
    case 'respond':
      return { type, id, fields: ctx.response ? autoMap(p, scope, ctx.response) : [] }
    case 'code':
      return { type, id, code: '// TODO' }
  }
}

// ---- diagnostics → selection ------------------------------------------------------

export type Selection =
  | { kind: 'service'; name: string }
  | { kind: 'topic'; name: string }
  | { kind: 'messages'; name?: string }
  | null

/** Which element a diagnostic pointer (/services/0/rpcs/1/...) belongs to. */
export function selectionForPath(p: Project, path: string): Selection {
  const [, root, index] = path.split('/')
  const i = Number(index)
  if (root === 'services' && p.services[i]) return { kind: 'service', name: p.services[i]!.name }
  if (root === 'topics' && p.topics[i]) return { kind: 'topic', name: p.topics[i]!.name }
  if (root === 'messages' && p.messages[i]) return { kind: 'messages', name: p.messages[i]!.name }
  return null
}
