import { autoMap } from '../model'
import { type Scope, typeLabel, typeOfField } from '../scope'
import { useStore } from '../store'
import type { Assignment, Expr } from '../types'
import { ExprEditor } from './ExprEditor'
import { Icon } from './Icon'
import { IconButton } from './ui'

/** One row per field of the target message: map each to a value, or leave it unset (default). */
export function AssignmentsEditor(props: {
  message: string
  assignments: Assignment[]
  scope: Scope
  /** Applies `fn` to this assignment list inside the project draft. */
  update: (fn: (list: Assignment[]) => void) => void
}) {
  const project = useStore((s) => s.project)
  const msg = project.messages.find((m) => m.name === props.message)
  if (!msg) return <p className="empty">Choisissez d’abord un message valide.</p>

  const byField = new Map(props.assignments.map((a) => [a.field, a]))
  const unknown = props.assignments.filter((a) => !msg.fields.some((f) => f.name === a.field))

  const setField = (field: string, value: Expr | null) =>
    props.update((list) => {
      const i = list.findIndex((a) => a.field === field)
      if (value === null) {
        if (i >= 0) list.splice(i, 1)
      } else if (i >= 0) {
        list[i]!.value = value
      } else {
        list.push({ field, value })
      }
    })

  const fillRest = () =>
    props.update((list) => {
      for (const a of autoMap(project, props.scope, props.message)) {
        if (!list.some((x) => x.field === a.field)) list.push(a)
      }
    })

  return (
    <div className="assignments">
      {msg.fields.length === 0 && <p className="empty">Le message {msg.name} n’a aucun champ.</p>}
      {msg.fields.map((f) => (
        <div className="assignment" key={f.name}>
          <div className="assignment-field" title={`${f.name}: ${typeLabel(typeOfField(f))}`}>
            <span className="mono">{f.name}</span>
            <small>{typeLabel(typeOfField(f))}</small>
          </div>
          <ExprEditor
            label={f.name}
            expr={byField.get(f.name)?.value ?? null}
            target={typeOfField(f)}
            scope={props.scope}
            allowUnset
            onChange={(v) => setField(f.name, v)}
          />
        </div>
      ))}
      {unknown.map((a) => (
        <div className="assignment stale" key={a.field}>
          <div className="assignment-field">
            <span className="mono">{a.field}</span>
            <small>champ inconnu</small>
          </div>
          <IconButton icon="trash" title={`Retirer ${a.field}`} danger onClick={() => setField(a.field, null)} />
        </div>
      ))}
      {msg.fields.length > 0 && (
        <button type="button" className="link-btn" onClick={fillRest}>
          <Icon name="sparkles" size={13} /> Remplir automatiquement les champs de même nom
        </button>
      )}
    </div>
  )
}
