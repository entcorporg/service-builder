// Editing helpers on the project document: factories and rename cascades.
// All mutate a draft (immer), never the live state.
import { candidates, columnType, fits, type Scope, type TypeRef, typeOfField } from './scope'
import type { Assignment, Column, Expr, ForeignKey, HttpMethod, Index, Job, Message, MessageKind, Project, Route, Service, Step, StepType, Table, UniqueConstraint } from './types'

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
    ...s.jobs.map((j) => ({ steps: j.steps, response: null as string | null })),
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
  // The steps that name a schema: a schema variable, the schema a SQL step converts rows into, the claims of a token.
  for (const h of allHandlers(p))
    walkSteps(h.steps, (st) => {
      if (st.type === 'let' && st.valueType.kind === 'message' && st.valueType.name === from) st.valueType.name = to
      if (st.type === 'sql' && st.into === from) st.into = to
      if (st.type === 'verifyJwt' && st.schema === from) st.schema = to
    })
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
    for (const c of s.consumers) {
      visit(c.steps, null)
      // The field that identifies the events of a consumer is one of the message of its topic.
      if (c.idempotentOn === from && p.topics.find((t) => t.name === c.topic)?.message === message) c.idempotentOn = to
    }
    for (const j of s.jobs) visit(j.steps, null)
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
    jobs: [],
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
  const table: Table = { name: uniqueName('new_table', service.tables.map((t) => t.name)), columns: [id], uniques: [], indexes: [], foreignKeys: [] }
  service.tables.push(table)
  return table
}

/** New columns are nullable: adding a required column without a default to a table that has rows would fail. */
export function addColumn(table: Table): Column {
  const col: Column = { name: uniqueName('column', table.columns.map((c) => c.name)), type: 'string', nullable: true, primaryKey: false, default: null }
  table.columns.push(col)
  return col
}

/** The column a soft delete sets. */
export const DELETED_AT = 'deleted_at'

/** Turns the soft delete of a table on or off. Turning it on adds the nullable timestamp `deleted_at` when it is missing. */
export function setSoftDelete(table: Table, on: boolean) {
  table.softDelete = on || undefined
  if (on && !table.columns.some((c) => c.name === DELETED_AT)) {
    table.columns.push({ name: DELETED_AT, type: 'timestamp', nullable: true, primaryKey: false, default: null })
  }
}

/** The columns that can scope a table to its tenant: required text, so that no row belongs to nobody. */
export function tenantCandidates(table: Table): string[] {
  return table.columns.filter((c) => !c.nullable && (c.type === 'string' || c.type === 'uuid')).map((c) => c.name)
}

/** Every top-level list of steps of the service (the SQL steps that name its tables are in there). */
function serviceSteps(svc: Service): Step[][] {
  return [...svc.rpcs.map((r) => r.steps), ...svc.routes.map((r) => r.steps), ...svc.consumers.map((c) => c.steps), ...svc.jobs.map((j) => j.steps)]
}

export function renameTable(p: Project, service: string, from: string, to: string) {
  const svc = p.services.find((s) => s.name === service)
  if (!svc) return
  for (const t of svc.tables) {
    if (t.name === from) t.name = to
    for (const fk of t.foreignKeys) if (fk.refTable === from) fk.refTable = to
  }
  for (const steps of serviceSteps(svc))
    walkSteps(steps, (st) => {
      if (st.type === 'sql' && st.table === from) st.table = to
    })
}

/** Renames a column, the constraints that name it and the SQL steps. Paths reading `row.from` are left to the validator. */
export function renameColumn(p: Project, service: string, table: string, from: string, to: string) {
  const svc = p.services.find((s) => s.name === service)
  if (!svc) return
  const swap = (x: string) => (x === from ? to : x)
  for (const t of svc.tables) {
    if (t.name === table) {
      for (const c of t.columns) if (c.name === from) c.name = to
      if (t.tenantColumn === from) t.tenantColumn = to
      for (const c of [...t.uniques, ...t.indexes, ...t.foreignKeys]) c.columns = c.columns.map(swap)
    }
    // A foreign key of any table of the service may point at the column.
    for (const fk of t.foreignKeys) if (fk.refTable === table) fk.refColumns = fk.refColumns.map(swap)
  }
  for (const steps of serviceSteps(svc))
    walkSteps(steps, (st) => {
      if (st.type !== 'sql' || st.table !== table) return
      st.columns = st.columns.map(swap)
      for (const c of st.where) c.column = swap(c.column)
      for (const v of st.values) v.field = swap(v.field)
      if (st.orderBy) st.orderBy = swap(st.orderBy)
    })
}

/**
 * A unique constraint and an index share one namespace in Postgres, which cuts names beyond 63 characters. The default
 * name starts with the table, which keeps tables apart; the validator catches the rare clash.
 */
function constraintName(table: Table, columns: string[], suffix: string): string {
  const taken = [...table.uniques, ...table.indexes, ...table.foreignKeys].map((c) => c.name)
  return uniqueName([table.name, ...columns, suffix].join('_').slice(0, 58), taken)
}

/** Constraints start on the first column, which the user then changes: an empty one would not be valid. */
export function addUnique(table: Table): UniqueConstraint {
  const columns = table.columns.slice(0, 1).map((c) => c.name)
  const unique = { name: constraintName(table, columns, 'uq'), columns }
  table.uniques.push(unique)
  return unique
}

export function addIndex(table: Table): Index {
  const columns = table.columns.slice(0, 1).map((c) => c.name)
  const index = { name: constraintName(table, columns, 'idx'), columns, unique: false }
  table.indexes.push(index)
  return index
}

/** Points a foreign key at another table: it references that table's primary key, pairing the columns it already had. */
export function retargetForeignKey(service: Service, table: Table, fk: ForeignKey, refTable: string) {
  const target = service.tables.find((t) => t.name === refTable)
  fk.refTable = refTable
  const pk = target?.columns.filter((c) => c.primaryKey).map((c) => c.name) ?? []
  fk.refColumns = pk.length ? pk : target?.columns.slice(0, 1).map((c) => c.name) ?? []
  fk.columns = fk.refColumns.map((_, i) => fk.columns[i] ?? table.columns[i]?.name ?? '')
}

/**
 * Starts a foreign key on the primary key of the first table that has one. Each referenced column is paired with a column of
 * the same type, preferring the same name (`shipments.order_id` → `orders.order_id`); a table never starts by pointing its
 * own key at itself.
 */
export function addForeignKey(service: Service, table: Table): ForeignKey | null {
  const target = service.tables.find((t) => t.columns.some((c) => c.primaryKey))
  if (!target) return null
  const refColumns = target.columns.filter((c) => c.primaryKey).map((c) => c.name)
  const used = new Set<string>()
  const columns = refColumns.map((r) => {
    const wanted = target.columns.find((c) => c.name === r)!.type
    const fits = table.columns.filter((c) => c.type === wanted && !used.has(c.name) && !(table === target && c.primaryKey))
    const name = (fits.find((c) => c.name === r) ?? fits[0])?.name ?? table.columns.find((c) => !used.has(c.name))?.name ?? ''
    used.add(name)
    return name
  })
  const fk: ForeignKey = {
    name: constraintName(table, columns, 'fk'),
    columns,
    refTable: target.name,
    refColumns,
    onDelete: 'noAction',
    onUpdate: 'noAction',
  }
  table.foreignKeys.push(fk)
  return fk
}

/** The API schemas a `jwks` step fills (RFC 7517), added when they are missing: a list of keys of five texts. */
export function ensureJwksSchemas(p: Project) {
  const text = (name: string) => ({ name, type: { kind: 'string' as const }, repeated: false, optional: false })
  if (!p.messages.some((m) => m.name === 'Jwk')) p.messages.push({ name: 'Jwk', kind: 'api', fields: ['kty', 'crv', 'alg', 'kid', 'x'].map(text) })
  if (!p.messages.some((m) => m.name === 'Jwks')) {
    p.messages.push({ name: 'Jwks', kind: 'api', fields: [{ name: 'keys', type: { kind: 'message', name: 'Jwk' }, repeated: true, optional: false }] })
  }
}

/** The API schema a `totpGenerate` step fills, added when it is missing: what to store, and what the client shows once. */
export function ensureTotpSchema(p: Project) {
  if (p.messages.some((m) => m.name === 'TotpSetup')) return
  const text = (name: string) => ({ name, type: { kind: 'string' as const }, repeated: false, optional: false })
  p.messages.push({ name: 'TotpSetup', kind: 'api', fields: ['sealed', 'otpauth_url', 'secret'].map(text) })
}

/** The API schema the `start` passkey steps fill, added when it is missing: the state to give back, and the browser's options. */
export function ensurePasskeySchema(p: Project) {
  if (p.messages.some((m) => m.name === 'PasskeyChallenge')) return
  const text = (name: string) => ({ name, type: { kind: 'string' as const }, repeated: false, optional: false })
  p.messages.push({ name: 'PasskeyChallenge', kind: 'api', fields: ['state_id', 'options'].map(text) })
}

/** The fields of a message that can tell its events apart: a single string, uuid, int32 or int64 (mirrors the validator). */
export function eventKeys(p: Project, message: string): string[] {
  const msg = p.messages.find((m) => m.name === message)
  return (msg?.fields ?? []).filter((f) => !f.repeated && !f.optional && ['string', 'uuid', 'int32', 'int64'].includes(f.type.kind)).map((f) => f.name)
}

/** A job every hour, which the user then names and draws; it starts empty. */
export function addJob(service: Service): Job {
  const job: Job = { name: uniqueName('job', service.jobs.map((j) => j.name)), everySecs: 3600, steps: [] }
  service.jobs.push(job)
  return job
}

export function addConsumer(p: Project, service: Service, topic = p.topics[0], base = 'on_event') {
  if (!topic) return null
  const consumer = {
    name: uniqueName(base, service.consumers.map((c) => c.name)),
    topic: topic.name,
    groupId: null as string | null,
    idempotentOn: null as string | null,
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

/** An expression as a short text, for summaries and previews. */
export function exprText(e: Expr): string {
  switch (e.kind) {
    case 'path':
      return e.path
    case 'raw':
      return e.code
    case 'now':
      return 'now()'
    case 'literal':
      return JSON.stringify(e.value)
  }
}

/** The error statuses a route is likely to answer with, for the steps that end a route (`fail`, a missing row). */
export const ERROR_STATUSES: [number, string][] = [
  [400, 'requête invalide'],
  [401, 'non authentifié'],
  [403, 'interdit'],
  [404, 'introuvable'],
  [409, 'conflit'],
  [410, 'disparu'],
  [422, 'non traitable'],
  [429, 'trop de requêtes'],
  [500, 'erreur interne'],
  [503, 'indisponible'],
]

export const STEP_LABELS: Record<StepType, { label: string; hint: string }> = {
  log: { label: 'Journal', hint: 'Écrire une ligne de log' },
  let: { label: 'Variable', hint: 'Calculer et nommer une valeur' },
  publish: { label: 'Publier un événement', hint: 'Mis en file (outbox) avec la transaction, puis envoyé sur le topic' },
  callRpc: { label: 'Appeler un RPC', hint: 'Appeler un autre service en gRPC' },
  branch: { label: 'Condition', hint: 'Si … alors … sinon …' },
  respond: { label: 'Répondre', hint: 'Renvoyer la réponse du RPC' },
  signJwt: { label: 'Signer un jeton', hint: 'Access token Ed25519 (EdDSA) : vos claims, plus émetteur, émission et expiration' },
  verifyJwt: { label: 'Vérifier un jeton', hint: 'Signature, émetteur et expiration ; les claims lus comme un schéma. Sinon 401' },
  jwks: { label: 'Publier les clés', hint: 'Le JWKS : les clés publiques qui permettent de vérifier un jeton, jamais la graine' },
  totpGenerate: { label: 'Activer le 2FA', hint: 'Secret TOTP chiffré à stocker, URI otpauth:// pour le QR code' },
  totpVerify: { label: 'Vérifier un code 2FA', hint: 'Fenêtre ±30 s, un code ne sert qu’une fois (anti-rejeu). Sinon 401' },
  recoveryCodes: { label: 'Codes de récupération', hint: 'XXXXX-XXXXX, à montrer une fois ; seules leurs empreintes sont stockées' },
  recoveryCodeUse: { label: 'Utiliser un code de récupération', hint: 'Valable une seule fois, avec ou sans tiret. Sinon 401' },
  passkeyRegisterStart: { label: 'Passkey : début d’enregistrement', hint: 'Un défi pour le navigateur (WebAuthn) ; le compte est déjà authentifié' },
  passkeyRegisterFinish: { label: 'Passkey : fin d’enregistrement', hint: 'Vérifie la réponse de l’appareil, une seule fois, et stocke la passkey. Sinon 400' },
  passkeyLoginStart: { label: 'Passkey : début de connexion', hint: 'Un défi pour les passkeys du compte. Sans passkey : 401' },
  passkeyLoginFinish: { label: 'Passkey : fin de connexion', hint: 'Vérifie la réponse et donne le compte. Sinon 401' },
  imageSniff: { label: 'Reconnaître une image', hint: 'PNG ou JPEG d’après les premiers octets, jamais le Content-Type du client ; pas de SVG. Sinon 400' },
  objectPut: { label: 'Stocker un objet', hint: 'Dans un bucket privé (S3 : Garage, MinIO ou AWS). L’objet d’abord, la base ensuite' },
  objectGet: { label: 'Lire un objet', hint: 'En base64 ; rien s’il n’existe pas. Stockage injoignable : 503' },
  objectDelete: { label: 'Supprimer un objet', hint: 'La base d’abord, l’objet ensuite. Un objet absent n’est pas une erreur' },
  encrypt: { label: 'Chiffrer', hint: 'AES-256-GCM, pour ce qui est stocké (un secret TOTP, un jeton)' },
  decrypt: { label: 'Déchiffrer', hint: 'Le texte qu’un bloc « Chiffrer » a scellé' },
  verifyHmac: { label: 'Vérifier une signature', hint: 'Webhook signé (HMAC-SHA256 du corps brut), sans session : toujours la première étape' },
  rateLimit: { label: 'Limiter le débit', hint: 'Au-delà de N essais par fenêtre : 429 et Retry-After' },
  hashPassword: { label: 'Hacher un mot de passe', hint: 'argon2id, hors du runtime async : le texte à stocker' },
  verifyPassword: { label: 'Vérifier un mot de passe', hint: 'Vrai ou faux ; un compte inconnu coûte le même temps' },
  randomToken: { label: 'Jeton aléatoire', hint: '256 bits en hexadécimal : session, e-mail, réinitialisation' },
  sha256: { label: 'Empreinte SHA-256', hint: 'Ce qu’on stocke d’un jeton, en hexadécimal' },
  fail: { label: 'Refuser', hint: 'Arrêter la route avec un statut d’erreur HTTP (400, 401, 404, 429…)' },
  sql: { label: 'SQL', hint: 'Lire, insérer, modifier ou supprimer dans une table du service' },
  code: { label: 'Code Rust', hint: 'Insérer du Rust écrit à la main' },
}

/**
 * The text in scope that a new step most likely reads: a field whose name holds one of `hints`, else (unless `onlyNamed`)
 * the first text, else `fallback`. `optional` looks for optional text, like a stored hash. A name holding one of `avoid`
 * is never picked by hint: the password to check is not the hash it is checked against.
 */
function firstText(
  p: Project,
  scope: Scope,
  hints: string[],
  { fallback = { kind: 'literal', value: '' } as Expr, optional = false, avoid = [] as string[], onlyNamed = false } = {},
): Expr {
  const target = { type: { kind: 'string' }, repeated: false, optional } as TypeRef
  const options = candidates(p, scope).filter((c) => c.path !== 'input' && fits(c.type, c.bare, target))
  const has = (c: { path: string }, words: string[]) => words.some((w) => c.path.toLowerCase().includes(w))
  const named = options.find((c) => has(c, hints) && !has(c, avoid))
  const pick = named ?? (optional || onlyNamed ? undefined : options[0])
  return pick ? { kind: 'path', path: pick.path } : fallback
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
        optional: false,
        repeated: false,
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
      return { type, id, fields: ctx.response ? autoMap(p, scope, ctx.response) : [], status: null, cookies: [] }
    case 'signJwt':
      return {
        type,
        id,
        keyEnv: 'JWT_SIGNING_KEY',
        issuer: ctx.service.toLowerCase(),
        ttlSecs: 300,
        claims: [{ field: 'sub', value: firstText(p, scope, ['sub', 'account', 'user', 'email', 'id']) }],
        bind: uniqueName('access_token', Object.keys(scope)),
      }
    case 'verifyJwt': {
      // The schema that lists the claims to read: one that looks like it, else any API schema.
      const api = p.messages.filter((m) => m.kind === 'api')
      const schema = (api.find((m) => /claims/i.test(m.name)) ?? api[0])?.name ?? ''
      return {
        type,
        id,
        token: firstText(p, scope, ['token', 'authorization']),
        bearer: false,
        source: { kind: 'keys', keyEnvs: ['JWT_SIGNING_KEY'] },
        issuer: ctx.service.toLowerCase(),
        schema,
        bind: uniqueName('claims', Object.keys(scope)),
      }
    }
    case 'jwks':
      return { type, id, keyEnvs: ['JWT_SIGNING_KEY'], bind: uniqueName('published', Object.keys(scope)) }
    case 'totpGenerate':
      return {
        type,
        id,
        keyEnv: 'TOTP_ENCRYPTION_KEY',
        issuer: ctx.service,
        account: firstText(p, scope, ['email', 'account', 'user', 'id']),
        bind: uniqueName('setup', Object.keys(scope)),
      }
    case 'totpVerify':
      return {
        type,
        id,
        keyEnv: 'TOTP_ENCRYPTION_KEY',
        sealed: firstText(p, scope, ['sealed', 'secret']),
        code: firstText(p, scope, ['code', 'otp']),
        account: firstText(p, scope, ['account', 'user', 'id', 'email']),
      }
    case 'recoveryCodes':
      return { type, id, account: firstText(p, scope, ['account', 'user', 'id', 'email']), count: 8, bind: uniqueName('recovery_codes', Object.keys(scope)) }
    case 'recoveryCodeUse':
      return {
        type,
        id,
        account: firstText(p, scope, ['account', 'user', 'id', 'email']),
        code: firstText(p, scope, ['recovery', 'code']),
      }
    case 'passkeyRegisterStart':
      return {
        type,
        id,
        account: firstText(p, scope, ['sub', 'account', 'user', 'id']),
        name: firstText(p, scope, ['name', 'email']),
        bind: uniqueName('challenge', Object.keys(scope)),
      }
    case 'passkeyRegisterFinish':
      return {
        type,
        id,
        stateId: firstText(p, scope, ['state']),
        credential: firstText(p, scope, ['credential', 'answer']),
        account: firstText(p, scope, ['sub', 'account', 'user', 'id']),
      }
    case 'passkeyLoginStart':
      return { type, id, account: firstText(p, scope, ['account', 'email', 'user']), bind: uniqueName('challenge', Object.keys(scope)) }
    case 'passkeyLoginFinish':
      return {
        type,
        id,
        stateId: firstText(p, scope, ['state']),
        credential: firstText(p, scope, ['credential', 'answer']),
        bind: uniqueName('account', Object.keys(scope)),
      }
    case 'imageSniff':
      return { type, id, input: firstText(p, scope, ['image', 'photo', 'avatar', 'file']), allow: ['png', 'jpeg'], maxBytes: 2 * 1024 * 1024, bind: uniqueName('content_type', Object.keys(scope)) }
    case 'objectPut':
      return {
        type,
        id,
        bucketEnv: 'AVATAR_BUCKET',
        key: firstText(p, scope, ['sub', 'account', 'user', 'id']),
        body: firstText(p, scope, ['image', 'photo', 'avatar', 'file']),
        contentType: firstText(p, scope, ['content_type', 'mime']),
      }
    case 'objectGet':
      return { type, id, bucketEnv: 'AVATAR_BUCKET', key: firstText(p, scope, ['sub', 'account', 'user', 'id']), bind: uniqueName('object', Object.keys(scope)) }
    case 'objectDelete':
      return { type, id, bucketEnv: 'AVATAR_BUCKET', key: firstText(p, scope, ['sub', 'account', 'user', 'id']) }
    case 'encrypt':
      return { type, id, keyEnv: 'ENCRYPTION_KEY', input: firstText(p, scope, ['secret', 'text']), bind: uniqueName('sealed', Object.keys(scope)) }
    case 'decrypt':
      return { type, id, keyEnv: 'ENCRYPTION_KEY', input: firstText(p, scope, ['sealed', 'enc']), bind: uniqueName('opened', Object.keys(scope)) }
    case 'verifyHmac':
      return { type, id, header: 'Stripe-Signature', secretEnv: 'STRIPE_WEBHOOK_SECRET', toleranceSecs: 300 }
    case 'rateLimit':
      // Counted per e-mail when the request has one: the usual guard of a login.
      return { type, id, key: firstText(p, scope, ['email'], { fallback: { kind: 'literal', value: 'global' }, onlyNamed: true }), max: 5, windowSecs: 900 }
    case 'randomToken':
      return { type, id, bind: uniqueName('token', Object.keys(scope)) }
    case 'sha256':
      return { type, id, input: firstText(p, scope, ['token'], { avoid: ['hash', 'digest'] }), bind: uniqueName('digest', Object.keys(scope)) }
    case 'hashPassword':
      return { type, id, input: firstText(p, scope, ['password'], { avoid: ['hash', 'digest'] }), bind: uniqueName('password_hash', Object.keys(scope)) }
    case 'verifyPassword':
      return {
        type,
        id,
        password: firstText(p, scope, ['password'], { avoid: ['hash', 'digest'] }),
        // The hash of the account read before, when there is one; else nothing (an unknown account).
        stored: firstText(p, scope, ['hash'], { fallback: { kind: 'literal', value: null }, optional: true }),
        bind: uniqueName('password_ok', Object.keys(scope)),
      }
    case 'fail':
      return { type, id, status: 400, message: 'bad request', retryAfter: null }
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
        missing: null,
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
