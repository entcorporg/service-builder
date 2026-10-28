// Mirror of the Rust model (crates/sb-model/src/project.rs). The JSON is camelCase.

export type ScalarKind = 'string' | 'bool' | 'int32' | 'int64' | 'float' | 'double' | 'bytes' | 'timestamp'

export type FieldType = { kind: ScalarKind } | { kind: 'message'; name: string }

export interface Field {
  name: string
  type: FieldType
  repeated: boolean
  optional: boolean
  tag?: number
}

export interface Message {
  name: string
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
  position: Position
}

export type Expr =
  | { kind: 'path'; path: string }
  | { kind: 'literal'; value: string | number | boolean }
  | { kind: 'raw'; code: string }

export type CmpOp = 'eq' | 'ne' | 'gt' | 'ge' | 'lt' | 'le'

export type Condition = { kind: 'compare'; left: Expr; op: CmpOp; right: Expr } | { kind: 'raw'; code: string }

export interface Assignment {
  field: string
  value: Expr
}

export type LogLevel = 'debug' | 'info' | 'warn' | 'error'

export type Step =
  | { type: 'log'; id: string; level: LogLevel; message: string }
  | { type: 'let'; id: string; name: string; valueType: FieldType; value: Expr }
  | { type: 'publish'; id: string; topic: string; payload: Assignment[] }
  | { type: 'callRpc'; id: string; service: string; method: string; request: Assignment[]; bind: string | null }
  | { type: 'branch'; id: string; condition: Condition; then: Step[]; otherwise: Step[] }
  | { type: 'respond'; id: string; fields: Assignment[] }
  | { type: 'code'; id: string; code: string }

export type StepType = Step['type']

export interface Rpc {
  name: string
  request: string
  response: string
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
  rpcs: Rpc[]
  consumers: Consumer[]
  position: Position
}

export interface Project {
  schemaVersion: number
  name: string
  broker: { bootstrapServers: string }
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

export interface ExportReport {
  dir: string
  files: number
}
