// Editing helpers on the project document: factories and rename cascades.
// All mutate a draft (immer), never the live state.
import { candidates, columnType, fits, type Scope, typeOfField } from './scope'
import type { Assignment, Column, HttpMethod, Message, MessageKind, Project, Route, Service, Step, StepType, Table } from './types'

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
    ...s.routes.map((r) => ({ steps: r.steps, response: r.response as string | null })),
    ...s.consumers.map((c) => ({ steps: c.steps, response: null as string | null })),
  ])
}

/** Services that have a consumer on `topic`. */
export function consumersOf(p: Project, topic: string): Service[] {
  return p.services.filter((s) => s.consumers.some((c) => c.topic === topic))
}

/** Whether `service` can be made a producer of `topic` (both exist, and it is not one yet). */
export function canProduce(p: Project, service: string, topic: string): boolean {
  const t = p.topics.find((x) => x.name === topic)
  return !!t && p.services.some((s) => s.name === service) && !t.producers.includes(service)
}

/** Declares `service` as an owner of `topic`, the only way to publish there. Drawing the arrow service → topic does this. */
export function declareProducer(p: Project, service: string, topic: string): boolean {
  if (!canProduce(p, service, topic)) return false
  p.topics.find((t) => t.name === topic)!.producers.push(service)
  return true
}

export function removeService(p: Project, name: string) {
  p.services = p.services.filter((s) => s.name !== name)
  for (const t of p.topics) t.producers = t.producers.filter((x) => x !== name)
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
  for (const s of p.services)
    for (const r of s.routes) {
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
  for (const t of p.topics) t.producers = t.producers.map((x) => (x === from ? to : x))
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
  // A `{param}` of a route path names a field of its request schema.
  for (const s of p.services) for (const r of s.routes) if (r.request === message) r.path = r.path.replace(`{${from}}`, `{${to}}`)
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
    for (const r of s.routes) visit(r.steps, r.response)
    for (const c of s.consumers) visit(c.steps, null)
  }
}

// ---- factories --------------------------------------------------------------------

export const KIND_LABELS: Record<MessageKind, { title: string; short: string; one: string; hint: string }> = {
  event: { title: 'Événements', short: 'Événements', one: 'Événement', hint: 'Charge utile d’un topic Kafka' },
  proto: { title: 'Proto (gRPC)', short: 'Proto', one: 'Proto', hint: 'Requêtes, réponses gRPC et types partagés' },
  api: { title: 'API', short: 'API', one: 'API', hint: 'Corps, paramètres et réponses des routes HTTP' },
}

export const KINDS: MessageKind[] = ['event', 'proto', 'api']

const DEFAULT_NAMES: Record<MessageKind, string> = { event: 'NewEvent', proto: 'NewMessage', api: 'NewApiSchema' }

export function addMessage(p: Project, base?: string, kind: MessageKind = 'proto'): Message {
  const msg: Message = {
    name: uniqueName(base ?? DEFAULT_NAMES[kind], p.messages.map((m) => m.name)),
    kind,
    fields: [{ name: 'id', type: { kind: 'string' }, repeated: false, optional: false, tag: 1 }],
  }
  p.messages.push(msg)
  return msg
}

/** Schemas a field of a `kind` schema can nest: events and proto share building blocks, API schemas live apart. */
export function nestable(p: Project, kind: MessageKind): Message[] {
  return p.messages.filter((m) => (m.kind === 'api') === (kind === 'api'))
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
    httpPort: null,
    rpcs: [],
    routes: [],
    tables: [],
    consumers: [],
    position,
  }
  p.services.push(svc)
  return svc
}

export function addTopic(p: Project, position: { x: number; y: number }) {
  const message = p.messages.find((m) => m.kind === 'event') ?? addMessage(p, 'Event', 'event')
  const topic = {
    name: uniqueName('new.topic', p.topics.map((t) => t.name)),
    message: message.name,
    partitions: 3,
    replicationFactor: 1,
    keyField: null as string | null,
    producers: [] as string[],
    position,
  }
  p.topics.push(topic)
  return topic
}

/** New RPC with its own request/response messages. */
export function addRpc(p: Project, service: Service) {
  const name = uniqueName('NewRpc', service.rpcs.map((r) => r.name))
  const request = addMessage(p, `${name}Request`, 'proto')
  const response = addMessage(p, `${name}Response`, 'proto')
  const rpc = { name, request: request.name, response: response.name, steps: [] as Step[] }
  service.rpcs.push(rpc)
  return rpc
}

export const METHODS: HttpMethod[] = ['get', 'post', 'put', 'patch', 'delete']

/** Success status a method gets by default. */
export const defaultStatus = (m: HttpMethod) => (m === 'post' ? 201 : 200)

/** The names between braces in a route path: the fields filled from the URL. */
export function pathParams(path: string): string[] {
  return [...path.matchAll(/\{([^{}/]*)\}/g)].map((m) => m[1]!)
}

/** New HTTP route with its own request/response API schemas, and an HTTP port for the service if it had none. */
export function addRoute(p: Project, service: Service): Route {
  const name = uniqueName('new_route', service.routes.map((r) => r.name))
  const pascal = name.split('_').map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join('')
  const request = addMessage(p, `${pascal}Request`, 'api')
  const response = addMessage(p, `${pascal}Response`, 'api')
  if (service.httpPort === null) service.httpPort = Math.max(8080, ...p.services.map((s) => s.httpPort ?? 0)) + 1
  const route: Route = {
    name,
    method: 'get',
    path: `/${name.replace(/_/g, '-')}`,
    request: request.name,
    response: response.name,
    status: 200,
    steps: [],
  }
  service.routes.push(route)
  return route
}

// ---- tables -----------------------------------------------------------------------

export function addTable(service: Service): Table {
  const id: Column = { name: 'id', type: 'uuid', nullable: false, primaryKey: true, default: 'gen_random_uuid()' }
  const table: Table = { name: uniqueName('new_table', service.tables.map((t) => t.name)), columns: [id] }
  service.tables.push(table)
  return table
}

/** New columns are nullable: adding a required column without a default to a table that has rows would fail. */
export function addColumn(table: Table): Column {
  const col: Column = { name: uniqueName('column', table.columns.map((c) => c.name)), type: 'string', nullable: true, primaryKey: false, default: null }
  table.columns.push(col)
  return col
}

/** Every top-level list of steps of the service (the SQL steps that name its tables are in there). */
function serviceSteps(svc: Service): Step[][] {
  return [...svc.rpcs.map((r) => r.steps), ...svc.routes.map((r) => r.steps), ...svc.consumers.map((c) => c.steps)]
}

export function renameTable(p: Project, service: string, from: string, to: string) {
  const svc = p.services.find((s) => s.name === service)
  if (!svc) return
  for (const t of svc.tables) if (t.name === from) t.name = to
  for (const steps of serviceSteps(svc))
    walkSteps(steps, (st) => {
      if (st.type === 'sql' && st.table === from) st.table = to
    })
}

/** Renames a column and the SQL steps that name it. Paths reading `row.from` are left to the validator. */
export function renameColumn(p: Project, service: string, table: string, from: string, to: string) {
  const svc = p.services.find((s) => s.name === service)
  if (!svc) return
  for (const c of svc.tables.find((t) => t.name === table)?.columns ?? []) if (c.name === from) c.name = to
  const swap = (x: string) => (x === from ? to : x)
  for (const steps of serviceSteps(svc))
    walkSteps(steps, (st) => {
      if (st.type !== 'sql' || st.table !== table) return
      st.columns = st.columns.map(swap)
      for (const c of st.where) c.column = swap(c.column)
      for (const v of st.values) v.field = swap(v.field)
      if (st.orderBy) st.orderBy = swap(st.orderBy)
    })
}

export function addConsumer(p: Project, service: Service, topic = p.topics[0], base = 'on_event') {
  if (!topic) return null
  const consumer = {
    name: uniqueName(base, service.consumers.map((c) => c.name)),
    topic: topic.name,
    groupId: null as string | null,
    steps: [] as Step[],
  }
  service.consumers.push(consumer)
  return consumer
}

/** `orders.created` → `on_orders_created`. */
function consumerNameFor(topic: string) {
  return `on_${topic.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '')}`
}

/** Whether `service` can start listening to `topic` (both exist, and it does not already). */
export function canConsume(p: Project, service: string, topic: string): boolean {
  const svc = p.services.find((s) => s.name === service)
  return !!svc && p.topics.some((t) => t.name === topic) && !svc.consumers.some((c) => c.topic === topic)
}

/** Drawing an edge topic → service creates a consumer on that service. */
export function connectConsumer(p: Project, service: string, topic: string) {
  if (!canConsume(p, service, topic)) return null
  return addConsumer(p, p.services.find((s) => s.name === service)!, p.topics.find((t) => t.name === topic), consumerNameFor(topic))
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
  publish: { label: 'Publier un événement', hint: 'Mis en file (outbox) avec la transaction, puis envoyé sur le topic' },
  callRpc: { label: 'Appeler un RPC', hint: 'Appeler un autre service en gRPC' },
  branch: { label: 'Condition', hint: 'Si … alors … sinon …' },
  respond: { label: 'Répondre', hint: 'Renvoyer la réponse du RPC' },
  sql: { label: 'SQL', hint: 'Lire, insérer, modifier ou supprimer dans une table du service' },
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
      // A topic this service already produces, else the first one (the caller then declares the service as producer).
      const topic = p.topics.find((t) => t.producers.includes(ctx.service)) ?? p.topics[0]
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
    case 'sql': {
      const table = p.services.find((x) => x.name === ctx.service)?.tables[0]
      // Reads the row whose key is already in scope (`input.order_id` for a table keyed by `order_id`).
      const key = table?.columns.find((c) => c.primaryKey)
      const match = key && candidates(p, scope).find((c) => c.path.split('.').at(-1) === key.name && c.path !== 'input' && fits(c.type, c.bare, columnType(key)))
      return {
        type,
        id,
        op: 'select',
        table: table?.name ?? '',
        bind: uniqueName(table ? table.name.replace(/s$/, '') || 'row' : 'row', Object.keys(scope)),
        columns: table?.columns.map((c) => c.name) ?? [],
        many: false,
        into: null,
        where: key && match ? [{ column: key.name, op: 'eq', value: { kind: 'path', path: match.path } }] : [],
        values: [],
        orderBy: null,
        descending: false,
        limit: null,
      }
    }
    case 'code':
      return { type, id, code: '// TODO' }
  }
}

// ---- diagnostics → selection ------------------------------------------------------

export type Selection =
  | { kind: 'service'; name: string }
  | { kind: 'topic'; name: string }
  | { kind: 'route'; service: string; route: string }
  | { kind: 'messages'; name?: string }
  | null

/** Which element a diagnostic pointer (/services/0/rpcs/1/...) belongs to. */
export function selectionForPath(p: Project, path: string): Selection {
  const [, root, index, sub, subIndex] = path.split('/')
  const i = Number(index)
  if (root === 'services' && sub === 'routes' && p.services[i]?.routes[Number(subIndex)]) {
    return { kind: 'route', service: p.services[i]!.name, route: p.services[i]!.routes[Number(subIndex)]!.name }
  }
  if (root === 'services' && p.services[i]) return { kind: 'service', name: p.services[i]!.name }
  if (root === 'topics' && p.topics[i]) return { kind: 'topic', name: p.topics[i]!.name }
  if (root === 'messages' && p.messages[i]) return { kind: 'messages', name: p.messages[i]!.name }
  return null
}
