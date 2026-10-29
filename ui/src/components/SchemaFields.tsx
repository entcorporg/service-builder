import { nestable, nextTag, renameField } from '../model'
import { SCALARS } from '../scope'
import { useStore } from '../store'
import type { Field, FieldType, MessageKind } from '../types'
import { CommitInput, IconButton } from './ui'

// API schemas travel as JSON: bytes and timestamp have no JSON form yet.
const TYPE_OPTIONS = (kind: MessageKind): string[] => (kind === 'api' ? [...SCALARS] : [...SCALARS, 'bytes', 'timestamp'])

export function typeValue(t: FieldType): string {
  return t.kind === 'message' ? `msg:${t.name}` : t.kind
}

export function parseType(v: string): FieldType {
  return v.startsWith('msg:') ? { kind: 'message', name: v.slice(4) } : ({ kind: v } as FieldType)
}

export function FieldRow(props: {
  field: Field
  /** Kind of the schema owning the field: it decides which types and schemas can be used. */
  kind: MessageKind
  tag: number
  onRename: (v: string) => void
  onChange: (fn: (f: Field) => void) => void
  onDelete: () => void
  /** Where the value comes from, shown after the name (HTTP request fields). */
  origin?: string
}) {
  const project = useStore((s) => s.project)
  const messages = nestable(project, props.kind)
  const f = props.field
  const isMsg = f.type.kind === 'message'
  return (
    <div className="field-row">
      <span className="tag" title="Numéro de champ protobuf, stable">#{props.tag}</span>
      <CommitInput value={f.name} mono label="Nom du champ" onCommit={props.onRename} />
      <select aria-label="Type du champ" value={typeValue(f.type)} onChange={(e) => props.onChange((x) => { x.type = parseType(e.target.value); if (x.type.kind === 'message') x.optional = false })}>
        <optgroup label="Types simples">
          {TYPE_OPTIONS(props.kind).map((k) => (
            <option key={k} value={k}>{k}</option>
          ))}
        </optgroup>
        {messages.length > 0 && (
          <optgroup label="Schémas">
            {messages.map((m) => (
              <option key={m.name} value={`msg:${m.name}`}>{m.name}</option>
            ))}
          </optgroup>
        )}
        {!TYPE_OPTIONS(props.kind).includes(f.type.kind) && f.type.kind !== 'message' && (
          <option value={typeValue(f.type)}>{f.type.kind} (non supporté ici)</option>
        )}
        {f.type.kind === 'message' && !messages.some((m) => m.name === (f.type as { name: string }).name) && (
          <option value={typeValue(f.type)}>
            {(f.type as { name: string }).name} ({project.messages.some((m) => m.name === (f.type as { name: string }).name) ? 'type incompatible' : 'inconnu'})
          </option>
        )}
      </select>
      <label className="check" title="Liste">
        <input type="checkbox" checked={f.repeated} onChange={(e) => props.onChange((x) => { x.repeated = e.target.checked; if (e.target.checked) x.optional = false })} />
        []
      </label>
      <label className="check" title="Optionnel (présence suivie)">
        <input type="checkbox" checked={f.optional} disabled={isMsg || f.repeated} onChange={(e) => props.onChange((x) => (x.optional = e.target.checked))} />
        ?
      </label>
      <IconButton icon="trash" title={`Supprimer ${f.name}`} danger onClick={props.onDelete} />
      {props.origin && <span className="origin" title="D’où vient la valeur dans la requête HTTP">{props.origin}</span>}
    </div>
  )
}


/** Picks a schema of one kind: events for a topic, proto for a gRPC request or response. */
export function MessageSelect(props: { kind: MessageKind; value: string; onChange: (v: string) => void }) {
  const messages = useStore((s) => s.project.messages).filter((m) => m.kind === props.kind)
  return (
    <select aria-label="Message" value={props.value} onChange={(e) => props.onChange(e.target.value)}>
      {!messages.some((m) => m.name === props.value) && <option value={props.value}>{props.value || 'Choisir…'}</option>}
      {messages.map((m) => (
        <option key={m.name}>{m.name}</option>
      ))}
    </select>
  )
}

/** Rows of the fields of `project.messages[index]`, with the add button. */
export function SchemaFields(props: { index: number; origin?: (field: Field) => string | undefined }) {
  const msg = useStore((s) => s.project.messages[props.index])
  const edit = useStore((s) => s.edit)
  if (!msg) return null
  return (
    <>
      <div className="fields">
        {msg.fields.map((f, fi) => (
          <FieldRow
            key={fi}
            field={f}
            kind={msg.kind}
            tag={f.tag ?? fi + 1}
            origin={props.origin?.(f)}
            onRename={(v) => edit((p) => renameField(p, msg.name, f.name, v))}
            onChange={(fn) => edit((p) => fn(p.messages[props.index]!.fields[fi]!))}
            onDelete={() => edit((p) => p.messages[props.index]!.fields.splice(fi, 1))}
          />
        ))}
      </div>
      <button
        type="button"
        className="link-btn"
        onClick={() =>
          edit((p) => {
            const m = p.messages[props.index]!
            const used = m.fields.map((x) => x.name)
            let n = m.fields.length + 1
            while (used.includes(`field_${n}`)) n++
            m.fields.push({ name: `field_${n}`, type: { kind: 'string' }, repeated: false, optional: false, tag: nextTag(m) })
          })
        }
      >
        + Ajouter un champ
      </button>
    </>
  )
}
