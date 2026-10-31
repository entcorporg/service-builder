import { nestable, nextTag, renameField } from '../model'
import { SCALARS } from '../scope'
import { useStore } from '../store'
import type { Field, FieldType, MessageKind, RequestSource, Rule } from '../types'
import { CommitInput, IconButton, NumberInput } from './ui'

// API schemas travel as JSON: bytes and timestamp have no JSON form yet.
const TYPE_OPTIONS = (kind: MessageKind): string[] => (kind === 'api' ? [...SCALARS] : [...SCALARS, 'bytes', 'timestamp'])

export function typeValue(t: FieldType): string {
  return t.kind === 'message' ? `msg:${t.name}` : t.kind
}

export function parseType(v: string): FieldType {
  return v.startsWith('msg:') ? { kind: 'message', name: v.slice(4) } : ({ kind: v } as FieldType)
}

const RULES: { rule: Rule['rule']; label: string; make: () => Rule }[] = [
  { rule: 'length', label: 'Longueur', make: () => ({ rule: 'length', min: 1, max: 128 }) },
  { rule: 'email', label: 'E-mail', make: () => ({ rule: 'email' }) },
  { rule: 'siren', label: 'SIREN', make: () => ({ rule: 'siren' }) },
]

/** The rules of a text field of an API schema: what a request must satisfy, checked before any step runs. */
function FieldRules(props: { field: Field; onChange: (fn: (f: Field) => void) => void }) {
  const rules = props.field.rules ?? []
  const left = RULES.filter((r) => !rules.some((x) => x.rule === r.rule))
  const set = (fn: (rules: Rule[]) => void) =>
    props.onChange((f) => {
      const next = [...(f.rules ?? [])]
      fn(next)
      f.rules = next.length > 0 ? next : undefined
    })
  return (
    <div className="flex flex-wrap items-center gap-2 pb-1.5 pl-9 text-xs" role="group" aria-label={`Règles de ${props.field.name}`}>
      <span className="text-2xs tracking-[0.7px] text-faint uppercase">Règles</span>
      {rules.map((r, i) => (
        <span key={r.rule} className="inline-flex items-center gap-1.5 rounded-md border border-line bg-panel-2 py-0.5 pr-0.5 pl-2">
          {r.rule === 'length' ? (
            <>
              longueur
              <span className="w-14"><NumberInput label={`Longueur minimale de ${props.field.name}`} min={0} value={r.min} onChange={(n) => set((x) => (x[i] = { ...r, min: n }))} /></span>
              à
              <span className="w-14"><NumberInput label={`Longueur maximale de ${props.field.name}`} min={0} value={r.max} onChange={(n) => set((x) => (x[i] = { ...r, max: n }))} /></span>
              <span className="text-faint" title="Des caractères, pas des octets ; le texte est rogné avant d’être compté">car.</span>
            </>
          ) : (
            RULES.find((x) => x.rule === r.rule)!.label
          )}
          <IconButton icon="x" size={11} title={`Retirer la règle ${r.rule} de ${props.field.name}`} onClick={() => set((x) => void x.splice(i, 1))} />
        </span>
      ))}
      {left.length > 0 && (
        <select aria-label={`Ajouter une règle à ${props.field.name}`} value="" onChange={(e) => set((x) => void x.push(RULES.find((r) => r.rule === e.target.value)!.make()))}>
          <option value="">+ règle…</option>
          {left.map((r) => (
            <option key={r.rule} value={r.rule}>{r.label}</option>
          ))}
        </select>
      )}
    </div>
  )
}

const SOURCES: { value: '' | RequestSource['from']; label: string; name: string }[] = [
  { value: '', label: 'Corps / query', name: '' },
  { value: 'header', label: 'En-tête', name: 'Authorization' },
  { value: 'cookie', label: 'Cookie', name: 'session' },
]

/** Where a route reads a required text field of its request from: the body, or a header or a cookie that always
 * replaces what the client sent in the body. */
function FieldSource(props: { field: Field; onChange: (fn: (f: Field) => void) => void }) {
  const source = props.field.source
  return (
    <div className="flex flex-wrap items-center gap-2 pb-1.5 pl-9 text-xs" role="group" aria-label={`Origine de ${props.field.name}`}>
      <span className="text-2xs tracking-[0.7px] text-faint uppercase">Lu depuis</span>
      <select
        aria-label={`Origine de ${props.field.name}`}
        value={source?.from ?? ''}
        onChange={(e) =>
          props.onChange((f) => {
            const next = SOURCES.find((s) => s.value === e.target.value)!
            f.source = next.value ? ({ from: next.value, name: next.name } as RequestSource) : undefined
          })
        }
      >
        {SOURCES.map((s) => (
          <option key={s.value} value={s.value}>{s.label}</option>
        ))}
      </select>
      {source && (
        <CommitInput
          value={source.name}
          mono
          label={`Nom ${source.from === 'header' ? 'de l’en-tête' : 'du cookie'} de ${props.field.name}`}
          onCommit={(name) => props.onChange((f) => void (f.source = { ...f.source!, name }))}
        />
      )}
      {source && <span className="text-faint">remplacé à chaque requête : le corps ne peut pas le fournir</span>}
    </div>
  )
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
  const text = f.type.kind === 'string' || f.type.kind === 'uuid'
  return (
    <>
    <div className="field-row">
      <span className="tag" title="Numéro de champ protobuf, stable">#{props.tag}</span>
      <CommitInput value={f.name} mono label="Nom du champ" onCommit={props.onRename} />
      <select aria-label="Type du champ" value={typeValue(f.type)} onChange={(e) => props.onChange((x) => { x.type = parseType(e.target.value); if (x.type.kind === 'message') x.optional = false; if (x.type.kind !== 'string' && x.type.kind !== 'uuid') { x.rules = undefined; x.source = undefined } })}>
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
        <input type="checkbox" checked={f.repeated} onChange={(e) => props.onChange((x) => { x.repeated = e.target.checked; if (e.target.checked) { x.optional = false; x.source = undefined } })} />
        []
      </label>
      <label className="check" title="Optionnel (présence suivie)">
        <input type="checkbox" checked={f.optional} disabled={isMsg || f.repeated} onChange={(e) => props.onChange((x) => { x.optional = e.target.checked; if (e.target.checked) x.source = undefined })} />
        ?
      </label>
      <IconButton icon="trash" title={`Supprimer ${f.name}`} danger onClick={props.onDelete} />
      {props.origin && <span className="origin" title="D’où vient la valeur dans la requête HTTP">{props.origin}</span>}
    </div>
    {props.kind === 'api' && text && <FieldRules field={f} onChange={props.onChange} />}
    {props.kind === 'api' && props.origin !== undefined && text && !f.repeated && !f.optional && <FieldSource field={f} onChange={props.onChange} />}
    </>
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
