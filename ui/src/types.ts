// Mirror of the Rust model (crates/sb-model/src/project.rs). The JSON is camelCase.

export type ScalarKind = 'string' | 'bool' | 'int32' | 'int64' | 'float' | 'double' | 'bytes' | 'timestamp' | 'uuid'

export type FieldType = { kind: ScalarKind } | { kind: 'message'; name: string }

export interface Field {
  name: string
  type: FieldType
  repeated: boolean
  optional: boolean
  tag?: number
}

/** What a schema is for: Kafka payload, gRPC message / shared type, or HTTP API (not generated yet). */
export type MessageKind = 'event' | 'proto' | 'api'

export interface Message {
  name: string
  kind: MessageKind
  fields: Field[]
}

export interface Position {
  x: number
  y: number
}

export interface Topic {
  name: string
  message: string
  partitions: number
  replicationFactor: number
  keyField: string | null
  /** Services allowed to publish here: the owners of the event. */
  producers: string[]
  position: Position
}

export type Expr =
  | { kind: 'path'; path: string }
  /** `null` is an absent optional value, an array a list. */
  | { kind: 'literal'; value: string | number | boolean | null | unknown[] }
  | { kind: 'raw'; code: string }

export type CmpOp = 'eq' | 'ne' | 'gt' | 'ge' | 'lt' | 'le'

export type Condition = { kind: 'compare'; left: Expr; op: CmpOp; right: Expr } | { kind: 'raw'; code: string }

export interface Assignment {
  field: string
  value: Expr
}

export type LogLevel = 'debug' | 'info' | 'warn' | 'error'

export type ColumnType = 'string' | 'bool' | 'int32' | 'int64' | 'float' | 'double' | 'timestamp' | 'uuid' | 'jsonb'

export interface Column {
  name: string
  type: ColumnType
  nullable: boolean
  primaryKey: boolean
  /** SQL expression used when an insert gives no value (`now()`, `0`, `'new'`). */
  default: string | null
}

/** A named `UNIQUE` constraint: the columns together identify at most one row. */
export interface UniqueConstraint {
  name: string
  columns: string[]
}

/** A secondary index; the order of the columns matters. */
export interface Index {
  name: string
  columns: string[]
  unique: boolean
}

export type ReferentialAction = 'noAction' | 'restrict' | 'cascade' | 'setNull' | 'setDefault'

/** `columns[i]` references `refColumns[i]` of `refTable`, in the same service. */
export interface ForeignKey {
  name: string
  columns: string[]
  refTable: string
  refColumns: string[]
  onDelete: ReferentialAction
  onUpdate: ReferentialAction
}

/** A table of the database of a service. */
export interface Table {
  name: string
  columns: Column[]
  uniques: UniqueConstraint[]
  indexes: Index[]
  foreignKeys: ForeignKey[]
}

export type SqlOp = 'select' | 'insert' | 'update' | 'delete'

/** `column <op> value`; the conditions of a step are all required (AND). */
export interface SqlCondition {
  column: string
  op: CmpOp
  value: Expr
}

export type Step =
  | { type: 'log'; id: string; level: LogLevel; message: string }
  | { type: 'let'; id: string; name: string; valueType: FieldType; optional: boolean; repeated: boolean; value: Expr }
  | { type: 'publish'; id: string; topic: string; payload: Assignment[] }
  | { type: 'callRpc'; id: string; service: string; method: string; request: Assignment[]; bind: string | null }
  | { type: 'branch'; id: string; condition: Condition; then: Step[]; otherwise: Step[] }
  | { type: 'respond'; id: string; fields: Assignment[] }
  /** Ends an HTTP route with an error status; `retryAfter` (seconds) goes with 429 and 503. */
  | { type: 'fail'; id: string; status: number; message: string; retryAfter: Expr | null }
  | {
      type: 'sql'
      id: string
      op: SqlOp
      table: string
      /** Select: the variable receiving the row (or the list of rows with `many`). */
      bind: string | null
      /** Select: the columns read, which type the variable. */
      columns: string[]
      many: boolean
      /** Select: convert the rows into this schema. */
      into: string | null
      where: SqlCondition[]
      /** Insert and update: the value of each column (`field` is the column name). */
      values: Assignment[]
      orderBy: string | null
      descending: boolean
      limit: number | null
    }
  | { type: 'code'; id: string; code: string }

export type StepType = Step['type']

export interface Rpc {
  name: string
  request: string
  response: string
  steps: Step[]
}

export type HttpMethod = 'get' | 'post' | 'put' | 'patch' | 'delete'

/** An HTTP endpoint for the front. `{params}` of the path fill fields of `request`; the rest comes from the JSON body or the query. */
export interface Route {
  name: string
  method: HttpMethod
  path: string
  request: string
  response: string
  /** Success status: 200, 201 or 202. */
  status: number
  steps: Step[]
}

export interface Consumer {
  name: string
  topic: string
  groupId: string | null
  steps: Step[]
}

export interface Service {
  name: string
  port: number
  /** Port of the HTTP API, needed as soon as there is a route. */
  httpPort: number | null
  rpcs: Rpc[]
  routes: Route[]
  tables: Table[]
  consumers: Consumer[]
  position: Position
}

export interface Project {
  schemaVersion: number
  name: string
  broker: { bootstrapServers: string }
  /** Publish attempts before an event is moved to its `<topic>.dlq`. */
  outbox: { maxAttempts: number }
  messages: Message[]
  topics: Topic[]
  services: Service[]
}

export interface Diagnostic {
  severity: 'error' | 'warning'
  code: string
  message: string
  /** JSON pointer into the project, e.g. /services/0/rpcs/1/steps/2 */
  path: string
}

export interface GeneratedFile {
  path: string
  content: string
}

export interface Analysis {
  diagnostics: Diagnostic[]
  files: GeneratedFile[]
}

export interface McpTool {
  name: string
  description: string
}

export interface McpActivation {
  configPath: string
  serverName: string
  /** `stale`: an entry exists but starts another program (the app moved or was updated). */
  state: 'off' | 'on' | 'stale'
}

export interface McpInfo {
  command: string
  tools: McpTool[]
  /** Null while the project has no file: the server works on a saved project. */
  activation: McpActivation | null
}

export interface AgentSuggestion {
  title: string
  details: string
}

export interface AgentReport {
  id: number
  /** Seconds since the Unix epoch. */
  createdAt: number
  summary: string
  suggestions: AgentSuggestion[]
  status: 'pending' | 'opened'
}

/** What the agents using the MCP server reported, kept next to the project file. */
export interface AgentFeedback {
  enabled: boolean
  interactions: number
  reports: AgentReport[]
}

export interface ExportReport {
  dir: string
  files: number
}
