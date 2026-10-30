import { addForeignKey, addIndex, addUnique, retargetForeignKey } from '../model'
import { useStore } from '../store'
import type { ForeignKey, ReferentialAction, Table } from '../types'
import { Icon } from './Icon'
import { CommitInput, IconButton } from './ui'

const ACTIONS: { value: ReferentialAction; label: string; sql: string }[] = [
  { value: 'noAction', label: 'Refuser (fin de transaction)', sql: 'NO ACTION' },
  { value: 'restrict', label: 'Refuser tout de suite', sql: 'RESTRICT' },
  { value: 'cascade', label: 'Propager', sql: 'CASCADE' },
  { value: 'setNull', label: 'Mettre à NULL', sql: 'SET NULL' },
  { value: 'setDefault', label: 'Valeur par défaut', sql: 'SET DEFAULT' },
]

const CARD = 'grid gap-2 rounded-2xl border border-line bg-panel px-3 py-2.5'
const CHIP = 'flex cursor-pointer items-center gap-1.5 rounded-md border border-line bg-panel-2 px-2 py-0.5 font-mono text-xs text-text'
const LABEL = 'text-2xs tracking-[0.7px] whitespace-nowrap text-faint uppercase'

/**
 * Picks the columns of a constraint. The order is the order they were ticked: it is the order of an index, and of the
 * columns a foreign key pairs. A column that no longer exists stays listed so that it can be unticked.
 */
function ColumnPicker(props: { table: Table; selected: string[]; label: string; onChange: (columns: string[]) => void }) {
  const known = props.table.columns.map((c) => c.name)
  const names = [...known, ...props.selected.filter((c) => !known.includes(c))]
  return (
    <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label={props.label}>
      {names.map((name) => {
        const at = props.selected.indexOf(name)
        return (
          <label key={name} className={`${CHIP} ${!known.includes(name) ? 'border-err text-err' : at >= 0 ? 'border-accent text-white' : ''}`}>
            <input
              type="checkbox"
              aria-label={`${props.label} : ${name}`}
              checked={at >= 0}
              onChange={(e) => props.onChange(e.target.checked ? [...props.selected, name] : props.selected.filter((c) => c !== name))}
            />
            {name}
            {at >= 0 && props.selected.length > 1 && <sup className="text-accent">{at + 1}</sup>}
          </label>
        )
      })}
    </div>
  )
}

/** The diagnostics of one constraint, under it. */
function Problems(props: { prefix: string }) {
  const diagnostics = useStore((s) => s.analysis.diagnostics)
  const mine = diagnostics.filter((d) => d.path === props.prefix || d.path.startsWith(`${props.prefix}/`))
  if (mine.length === 0) return null
  return (
    <ul className="m-0 grid list-none gap-0.5 p-0 text-sm" aria-label="Problèmes de la contrainte">
      {mine.map((d, i) => (
        <li key={i} className={`flex items-center gap-1.5 ${d.severity === 'error' ? 'text-err' : 'text-warn'}`}>
          <Icon name={d.severity === 'error' ? 'error' : 'warn'} size={12} /> {d.message}
        </li>
      ))}
    </ul>
  )
}

function Heading(props: { title: string; add: string; onAdd: () => void; disabled?: boolean }) {
  return (
    <div className="flex items-center gap-3">
      <h3 className="m-0 text-xs tracking-[0.8px] text-muted uppercase">{props.title}</h3>
      <button type="button" className="link-btn" disabled={props.disabled} onClick={props.onAdd}>
        {props.add}
      </button>
    </div>
  )
}

/** One `columns[i] → refColumns[i]` pair of a foreign key. */
function ForeignKeyCard(props: { si: number; ti: number; fi: number; fk: ForeignKey }) {
  const { si, ti, fi, fk } = props
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  const service = project.services[si]!
  const table = service.tables[ti]!
  const target = service.tables.find((t) => t.name === fk.refTable)
  const change = (fn: (fk: ForeignKey, table: Table) => void) => edit((p) => fn(p.services[si]!.tables[ti]!.foreignKeys[fi]!, p.services[si]!.tables[ti]!))
  const options = (t: Table | undefined, current: string) => {
    const names = t?.columns.map((c) => c.name) ?? []
    return (names.includes(current) ? names : [current, ...names]).map((n) => <option key={n} value={n}>{n}</option>)
  }
  return (
    <div className={CARD} role="group" aria-label={`Clé étrangère ${fk.name}`}>
      <div className="flex items-center gap-2">
        <CommitInput value={fk.name} mono label="Nom de la clé étrangère" className="w-[260px]" onCommit={(v) => change((f) => (f.name = v))} />
        <span className="flex-1" />
        <IconButton icon="trash" title={`Supprimer la clé étrangère ${fk.name}`} danger onClick={() => edit((p) => void p.services[si]!.tables[ti]!.foreignKeys.splice(fi, 1))} />
      </div>
      <div className="flex items-center gap-2">
        <span className={LABEL}>Référence</span>
        <select aria-label="Table référencée" value={fk.refTable} onChange={(e) => edit((p) => retargetForeignKey(p.services[si]!, p.services[si]!.tables[ti]!, p.services[si]!.tables[ti]!.foreignKeys[fi]!, e.target.value))}>
          {!target && <option value={fk.refTable}>{fk.refTable}</option>}
          {service.tables.map((t) => <option key={t.name} value={t.name}>{t.name}</option>)}
        </select>
      </div>
      <div className="grid grid-cols-[minmax(0,1fr)_20px_minmax(0,1fr)_34px] items-center gap-2" role="group" aria-label="Colonnes associées">
        <span className={LABEL}>{table.name}</span>
        <span />
        <span className={LABEL}>{fk.refTable}</span>
        <span />
        {fk.columns.map((c, k) => (
          <div key={k} className="contents">
            <select aria-label={`Colonne ${k + 1}`} value={c} onChange={(e) => change((f) => (f.columns[k] = e.target.value))}>
              {options(table, c)}
            </select>
            <span className="text-center text-muted">→</span>
            <select aria-label={`Colonne référencée ${k + 1}`} value={fk.refColumns[k] ?? ''} onChange={(e) => change((f) => (f.refColumns[k] = e.target.value))}>
              {options(target, fk.refColumns[k] ?? '')}
            </select>
            <IconButton
              icon="trash"
              title={`Retirer la paire ${k + 1}`}
              danger
              disabled={fk.columns.length <= 1}
              onClick={() => change((f) => (f.columns.splice(k, 1), f.refColumns.splice(k, 1)))}
            />
          </div>
        ))}
      </div>
      <button type="button" className="link-btn justify-self-start" onClick={() => change((f, t) => (f.columns.push(t.columns[0]?.name ?? ''), f.refColumns.push(target?.columns[0]?.name ?? '')))}>
        + Ajouter une paire de colonnes
      </button>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5">
        {(['onDelete', 'onUpdate'] as const).map((when) => (
          <label key={when} className="flex items-center gap-2">
            <span className={LABEL}>{when === 'onDelete' ? 'À la suppression' : 'À la mise à jour'}</span>
            <select value={fk[when]} onChange={(e) => change((f) => (f[when] = e.target.value as ReferentialAction))}>
              {ACTIONS.map((a) => <option key={a.value} value={a.value}>{a.label}</option>)}
            </select>
          </label>
        ))}
      </div>
      <Problems prefix={`/services/${si}/tables/${ti}/foreignKeys/${fi}`} />
    </div>
  )
}

/**
 * The constraints of a table beyond its columns: composite `UNIQUE` constraints, indexes, and foreign keys, which can
 * name several columns. They are added to the database after the tables, so a foreign key may point at any table
 * of the service.
 */
export function TableConstraints(props: { si: number; ti: number }) {
  const { si, ti } = props
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  const service = project.services[si]!
  const table = service.tables[ti]!
  const canReference = service.tables.some((t) => t.columns.some((c) => c.primaryKey))
  const at = () => (p: typeof project) => p.services[si]!.tables[ti]!

  return (
    <div className="grid gap-[18px]">
      <section className="grid gap-2" aria-label="Contraintes UNIQUE">
        <Heading title="Contraintes UNIQUE" add="+ Ajouter une contrainte" onAdd={() => edit((p) => void addUnique(at()(p)))} />
        {table.uniques.map((u, i) => (
          <div key={i} className={CARD} role="group" aria-label={`Contrainte UNIQUE ${u.name}`}>
            <div className="flex items-center gap-2">
              <CommitInput value={u.name} mono label="Nom de la contrainte" className="w-[260px]" onCommit={(v) => edit((p) => (at()(p).uniques[i]!.name = v))} />
              <span className="flex-1" />
              <IconButton icon="trash" title={`Supprimer la contrainte ${u.name}`} danger onClick={() => edit((p) => void at()(p).uniques.splice(i, 1))} />
            </div>
            <ColumnPicker table={table} selected={u.columns} label="Colonnes uniques" onChange={(cols) => edit((p) => (at()(p).uniques[i]!.columns = cols))} />
            <Problems prefix={`/services/${si}/tables/${ti}/uniques/${i}`} />
          </div>
        ))}
      </section>

      <section className="grid gap-2" aria-label="Index">
        <Heading title="Index" add="+ Ajouter un index" onAdd={() => edit((p) => void addIndex(at()(p)))} />
        {table.indexes.map((x, i) => (
          <div key={i} className={CARD} role="group" aria-label={`Index ${x.name}`}>
            <div className="flex items-center gap-2">
              <CommitInput value={x.name} mono label="Nom de l’index" className="w-[260px]" onCommit={(v) => edit((p) => (at()(p).indexes[i]!.name = v))} />
              <label className="flex cursor-pointer items-center gap-1.5 text-sm text-muted" title="Un index unique refuse les doublons">
                <input type="checkbox" aria-label={`${x.name} est unique`} checked={x.unique} onChange={(e) => edit((p) => (at()(p).indexes[i]!.unique = e.target.checked))} />
                unique
              </label>
              <span className="flex-1" />
              <IconButton icon="trash" title={`Supprimer l’index ${x.name}`} danger onClick={() => edit((p) => void at()(p).indexes.splice(i, 1))} />
            </div>
            <ColumnPicker table={table} selected={x.columns} label="Colonnes de l’index" onChange={(cols) => edit((p) => (at()(p).indexes[i]!.columns = cols))} />
            <Problems prefix={`/services/${si}/tables/${ti}/indexes/${i}`} />
          </div>
        ))}
      </section>

      <section className="grid gap-2" aria-label="Clés étrangères">
        <Heading title="Clés étrangères" add="+ Ajouter une clé étrangère" disabled={!canReference} onAdd={() => edit((p) => void addForeignKey(p.services[si]!, at()(p)))} />
        {!canReference && <small className="text-faint">Une clé étrangère référence la clé primaire d’une table du service : définissez-en une d’abord.</small>}
        {table.foreignKeys.map((fk, i) => <ForeignKeyCard key={i} si={si} ti={ti} fi={i} fk={fk} />)}
      </section>
    </div>
  )
}
