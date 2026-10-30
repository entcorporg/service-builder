import { ERROR_STATUSES, exprText, uniqueName } from '../model'
import { columnType, type Scope, tableOf, typeLabel } from '../scope'
import { useStore } from '../store'
import type { CmpOp, Column, SqlCondition, SqlOp, Step } from '../types'
import { ExprEditor } from './ExprEditor'
import { Icon } from './Icon'
import { Empty, IconButton } from './ui'

type SqlStep = Extract<Step, { type: 'sql' }>

export const OP_LABELS: Record<SqlOp, string> = { select: 'Lire', insert: 'Insérer', update: 'Modifier', delete: 'Supprimer' }
const SQL_OPS: Record<CmpOp, string> = { eq: '=', ne: '<>', gt: '>', ge: '>=', lt: '<', le: '<=' }
const colLabel = (c: Column) => `${c.type}${c.nullable ? '?' : ''}`

/** `SELECT order_id, status FROM orders WHERE order_id = input.order_id LIMIT 1`, to read the step at a glance. */
export function sqlPreview(step: SqlStep): string {
  const where = step.where.length ? ` WHERE ${step.where.map((c) => `${c.column} ${SQL_OPS[c.op]} ${exprText(c.value)}`).join(' AND ')}` : ''
  const val = (a: SqlStep['values'][number]) => `${a.field} = ${exprText(a.value)}`
  const returning = step.columns.length > 0 ? ` RETURNING ${step.columns.join(', ')}` : ''
  switch (step.op) {
    case 'select':
      return `SELECT ${step.columns.join(', ') || '…'} FROM ${step.table}${where}${step.orderBy ? ` ORDER BY ${step.orderBy}${step.descending ? ' DESC' : ''}` : ''}${step.many ? (step.limit ? ` LIMIT ${step.limit}` : '') : ' LIMIT 1'}`
    case 'insert':
      return `INSERT INTO ${step.table} (${step.values.map((v) => v.field).join(', ')}) VALUES (${step.values.map((v) => exprText(v.value)).join(', ') || '…'})${returning}`
    case 'update':
      return `UPDATE ${step.table} SET ${step.values.map(val).join(', ') || '…'}${where}${returning}`
    case 'delete':
      return `DELETE FROM ${step.table}${where}${returning}`
  }
}

/**
 * The editor of a SQL step. It knows the tables of the service, so it can say what the step will give
 * the next ones: ticking columns types the variable (`order.status`), and the values it takes are typed
 * by their column.
 */
export function SqlBody(props: { step: SqlStep; update: (fn: (s: SqlStep) => void) => void; scope: Scope; service: string; http?: boolean }) {
  const { step, update: u, scope } = props
  const project = useStore((s) => s.project)
  const setView = useStore((s) => s.setView)
  const service = project.services.find((s) => s.name === props.service)
  const tables = service?.tables ?? []
  const table = tableOf(service, step.table)

  if (tables.length === 0) {
    return (
      <div className="stack">
        <Empty>Ce service n’a aucune table : concevez-les dans la vue Tables.</Empty>
        <button type="button" className="link-btn" onClick={() => setView('tables')}>
          <Icon name="table" size={13} /> Ouvrir la vue Tables
        </button>
      </div>
    )
  }

  // A select always reads rows; the other operations give them back only when asked (`RETURNING`).
  const reads = step.op === 'select' || step.columns.length > 0
  const dropResult = (s: SqlStep) => {
    s.columns = []
    s.bind = null
    s.many = false
    s.into = null
    s.missing = null
  }
  const setOp = (op: SqlOp) =>
    u((s) => {
      const was = s.op
      s.op = op
      if (op === 'select') {
        s.bind ||= uniqueName('row', Object.keys(scope))
        if (s.columns.length === 0) s.columns = table?.columns.map((c) => c.name) ?? []
      } else if (was === 'select') {
        // What a select read is not asked back from a write: it starts without a result.
        dropResult(s)
        s.orderBy = null
        s.limit = null
      }
      s.values = op === 'insert' || op === 'update' ? s.values : []
    })
  const setReturning = (on: boolean) =>
    u((s) => {
      if (!on) return dropResult(s)
      s.bind ||= uniqueName('row', Object.keys(scope))
      s.columns = table?.columns.map((c) => c.name) ?? []
    })
  const setTable = (name: string) =>
    u((s) => {
      s.table = name
      const t = tableOf(service, name)
      s.columns = s.op === 'select' || s.columns.length > 0 ? (t?.columns.map((c) => c.name) ?? []) : []
      s.where = []
      s.values = []
      s.orderBy = null
    })
  const toggleColumn = (name: string, on: boolean) =>
    u((s) => {
      // Keep the order of the table, not the order of the clicks.
      s.columns = (table?.columns ?? []).map((c) => c.name).filter((n) => (n === name ? on : s.columns.includes(n)))
    })
  const setCondition = (i: number, fn: (c: SqlCondition) => void) => u((s) => fn(s.where[i]!))
  const addCondition = () =>
    u((s) => {
      const col = table?.columns.find((c) => c.primaryKey) ?? table?.columns[0]
      if (col) s.where.push({ column: col.name, op: 'eq', value: { kind: 'raw', code: '' } })
    })

  const selectedColumns = (table?.columns ?? []).filter((c) => step.columns.includes(c.name))
  const schemas = project.messages.filter((m) => m.kind !== 'event')
  const required = step.op === 'update' || step.op === 'delete'

  return (
    <div className="stack sql">
      <div className="segmented" role="group" aria-label="Opération">
        {(Object.keys(OP_LABELS) as SqlOp[]).map((op) => (
          <button key={op} type="button" className={step.op === op ? 'active' : ''} aria-pressed={step.op === op} onClick={() => setOp(op)}>
            {OP_LABELS[op]}
          </button>
        ))}
      </div>

      <label className="form-row">
        <span>Table</span>
        <select aria-label="Table" value={step.table} onChange={(e) => setTable(e.target.value)}>
          {!table && <option value={step.table}>{step.table || 'Choisir…'}</option>}
          {tables.map((t) => (
            <option key={t.name}>{t.name}</option>
          ))}
        </select>
      </label>

      {table && step.op !== 'select' && (
        <label className="check" title="Donne la ligne écrite en retour, dans la même instruction (RETURNING)">
          <input type="checkbox" aria-label="Renvoyer la ligne" checked={step.columns.length > 0} onChange={(e) => setReturning(e.target.checked)} /> renvoyer la ligne (RETURNING)
        </label>
      )}

      {table && reads && (
        <>
          <div className="form-row">
            <span>Résultat</span>
            <input className="mono" aria-label="Nom de la variable" value={step.bind ?? ''} spellCheck={false} onChange={(e) => u((s) => (s.bind = e.target.value))} />
            <label className="check" title="Toutes les lignes qui correspondent (une liste) au lieu d’une seule">
              <input type="checkbox" checked={step.many} onChange={(e) => u((s) => (s.many = e.target.checked))} /> plusieurs
            </label>
          </div>

          <fieldset className="columns">
            <legend>{step.op === 'select' ? 'Colonnes à lire' : 'Colonnes renvoyées'}</legend>
            {table.columns.map((c) => (
              <label key={c.name} className="check column">
                <input type="checkbox" checked={step.columns.includes(c.name)} onChange={(e) => toggleColumn(c.name, e.target.checked)} />
                <span className="mono">{c.name}</span>
                <small>{colLabel(c)}</small>
                {c.primaryKey && <small className="pk">clé</small>}
              </label>
            ))}
          </fieldset>

          <label className="form-row">
            <span>Vers un schéma</span>
            <select aria-label="Convertir en schéma" value={step.into ?? ''} onChange={(e) => u((s) => (s.into = e.target.value || null))}>
              <option value="">aucun : une ligne typée par les colonnes</option>
              {schemas.map((m) => (
                <option key={m.name}>{m.name}</option>
              ))}
            </select>
          </label>
          {step.many && !step.into && <small className="muted warn">Une liste de lignes se lit à travers un schéma : choisissez-en un.</small>}

          <div className="prediction" aria-label="Variable produite">
            <Icon name="sparkles" size={13} />{' '}
            {step.bind ? (
              <>
                <code>{step.bind}</code> : {step.into ? `${step.many ? 'liste de ' : ''}${step.into}` : selectedColumns.length ? '' : 'aucune colonne'}
                {!step.into && selectedColumns.length > 0 && (
                  <span className="fields-line">
                    {selectedColumns.map((c) => (
                      <span key={c.name}>
                        <code>{`${step.bind}.${c.name}`}</code> {typeLabel(columnType(c))}
                      </span>
                    ))}
                  </span>
                )}
              </>
            ) : (
              'Nommez le résultat pour pouvoir le lire dans les étapes suivantes.'
            )}
          </div>
        </>
      )}

      {table && (step.op === 'insert' || step.op === 'update') && (
        <div className="assignments">
          <h5 className="mini-title">{step.op === 'insert' ? 'Valeurs à insérer' : 'Nouvelles valeurs'}</h5>
          {table.columns.map((c) => {
            const a = step.values.find((v) => v.field === c.name)
            const needed = step.op === 'insert' && c.nullable === false && !c.default?.trim()
            return (
              <div className="assignment" key={c.name}>
                <div className="assignment-field" title={`${c.name}: ${colLabel(c)}`}>
                  <span className="mono">{c.name}{needed && <b className="req" title="Obligatoire : ni nul ni valeur par défaut"> *</b>}</span>
                  <small>{colLabel(c)}{c.default ? ` · défaut ${c.default}` : ''}</small>
                </div>
                <ExprEditor
                  label={c.name}
                  expr={a?.value ?? null}
                  target={columnType(c)}
                  scope={scope}
                  allowUnset
                  allowNow={c.type === 'timestamp'}
                  onChange={(v) =>
                    u((s) => {
                      const i = s.values.findIndex((x) => x.field === c.name)
                      if (v === null) {
                        if (i >= 0) s.values.splice(i, 1)
                      } else if (i >= 0) s.values[i]!.value = v
                      else s.values.push({ field: c.name, value: v })
                    })
                  }
                />
              </div>
            )
          })}
        </div>
      )}

      {table && step.op !== 'insert' && (
        <div className="conditions">
          <h5 className="mini-title">Où {required && <span className="req" title="Obligatoire : sans condition, toutes les lignes seraient touchées">*</span>}</h5>
          {step.where.map((c, i) => {
            const col = table.columns.find((x) => x.name === c.column)
            return (
              <div className="sql-condition" key={i}>
                <select aria-label="Colonne" value={c.column} onChange={(e) => setCondition(i, (x) => (x.column = e.target.value))}>
                  {!col && <option value={c.column}>{c.column}</option>}
                  {table.columns.map((x) => (
                    <option key={x.name}>{x.name}</option>
                  ))}
                </select>
                <select aria-label="Opérateur" value={c.op} onChange={(e) => setCondition(i, (x) => (x.op = e.target.value as CmpOp))}>
                  {(Object.keys(SQL_OPS) as CmpOp[]).map((op) => (
                    <option key={op} value={op}>{SQL_OPS[op]}</option>
                  ))}
                </select>
                <ExprEditor label={`valeur de ${c.column}`} expr={c.value} target={col ? columnType(col) : null} scope={scope} allowNow={col?.type === 'timestamp'} onChange={(v) => v && setCondition(i, (x) => (x.value = v))} />
                <IconButton icon="trash" title="Retirer la condition" danger onClick={() => u((s) => void s.where.splice(i, 1))} />
              </div>
            )
          })}
          <button type="button" className="link-btn" onClick={addCondition}>+ Ajouter une condition</button>
        </div>
      )}

      {table && step.op === 'select' && step.many && (
        <div className="form-row">
          <span>Tri</span>
          <select aria-label="Trier par" value={step.orderBy ?? ''} onChange={(e) => u((s) => (s.orderBy = e.target.value || null))}>
            <option value="">aucun</option>
            {table.columns.map((c) => (
              <option key={c.name}>{c.name}</option>
            ))}
          </select>
          <label className="check"><input type="checkbox" checked={step.descending} onChange={(e) => u((s) => (s.descending = e.target.checked))} /> décroissant</label>
          <input type="number" min={1} aria-label="Limite" placeholder="limite" value={step.limit ?? ''} onChange={(e) => u((s) => (s.limit = e.target.value === '' ? null : Number(e.target.value)))} />
        </div>
      )}

      {table && reads && step.bind && !step.many && props.http && (
        <div className="stack" role="group" aria-label="Si aucune ligne">
          <label className="check" title="Un jeton inconnu, expiré ou déjà utilisé, un compte qui n’existe pas…">
            <input
              type="checkbox"
              aria-label="Répondre autrement qu’en 404"
              checked={step.missing !== null}
              onChange={(e) => u((s) => (s.missing = e.target.checked ? { status: 400, message: 'invalid request' } : null))}
            />{' '}
            si aucune ligne ne correspond, répondre autre chose que 404
          </label>
          {step.missing && (
            <div className="form-row">
              <select aria-label="Statut si aucune ligne" value={step.missing.status} onChange={(e) => u((s) => s.missing && (s.missing.status = Number(e.target.value)))}>
                {ERROR_STATUSES.map(([n, label]) => (
                  <option key={n} value={n}>{n} · {label}</option>
                ))}
              </select>
              <input aria-label="Message si aucune ligne" value={step.missing.message} placeholder="ce que le client reçoit" onChange={(e) => u((s) => s.missing && (s.missing.message = e.target.value))} />
            </div>
          )}
        </div>
      )}

      <pre className="sql-preview mono" aria-label="Requête SQL">{sqlPreview(step)}</pre>
    </div>
  )
}
