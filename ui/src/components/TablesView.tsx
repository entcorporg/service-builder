import { useState } from 'react'
import { addColumn, addTable, renameColumn, renameTable } from '../model'
import { useStore } from '../store'
import type { ColumnType, Project, Table } from '../types'
import { Icon } from './Icon'
import { TableConstraints } from './TableConstraints'
import { CommitInput, IconButton } from './ui'

const TYPES: ColumnType[] = ['string', 'bool', 'int32', 'int64', 'float', 'double', 'timestamp', 'uuid', 'jsonb']
const SQL_TYPE: Record<ColumnType, string> = {
  string: 'TEXT',
  bool: 'BOOLEAN',
  int32: 'INTEGER',
  int64: 'BIGINT',
  float: 'REAL',
  double: 'DOUBLE PRECISION',
  timestamp: 'TIMESTAMPTZ',
  uuid: 'UUID',
  jsonb: 'JSONB',
}

const ACTION_SQL = { noAction: 'NO ACTION', restrict: 'RESTRICT', cascade: 'CASCADE', setNull: 'SET NULL', setDefault: 'SET DEFAULT' }

/** The SQL the service applies at start-up for the table, to read the design as SQL: the table, then what is added to it. */
export function createTableSql(t: Table): string {
  const cols = t.columns.map((c) => `  ${c.name} ${SQL_TYPE[c.type]}${c.nullable ? '' : ' NOT NULL'}${c.default?.trim() ? ` DEFAULT ${c.default.trim()}` : ''}`)
  const pk = t.columns.filter((c) => c.primaryKey).map((c) => c.name)
  if (pk.length) cols.push(`  PRIMARY KEY (${pk.join(', ')})`)
  const alter = (name: string, def: string) => `ALTER TABLE ${t.name} ADD CONSTRAINT ${name} ${def};`
  return [
    `CREATE TABLE IF NOT EXISTS ${t.name} (\n${cols.join(',\n')}\n);`,
    ...t.uniques.map((u) => alter(u.name, `UNIQUE (${u.columns.join(', ')})`)),
    ...t.indexes.map((i) => `CREATE ${i.unique ? 'UNIQUE ' : ''}INDEX IF NOT EXISTS ${i.name} ON ${t.name} (${i.columns.join(', ')});`),
    ...t.foreignKeys.map((f) =>
      alter(f.name, `FOREIGN KEY (${f.columns.join(', ')}) REFERENCES ${f.refTable} (${f.refColumns.join(', ')}) ON DELETE ${ACTION_SQL[f.onDelete]} ON UPDATE ${ACTION_SQL[f.onUpdate]}`),
    ),
  ].join('\n')
}

/** SQL steps that read or write the table, to know what a change touches. */
function usages(project: Project, service: string, table: string): number {
  const svc = project.services.find((s) => s.name === service)
  if (!svc) return 0
  let n = 0
  const visit = (steps: import('../types').Step[]) => {
    for (const s of steps) {
      if (s.type === 'sql' && s.table === table) n++
      if (s.type === 'branch') {
        visit(s.then)
        visit(s.otherwise)
      }
    }
  }
  for (const r of svc.rpcs) visit(r.steps)
  for (const r of svc.routes) visit(r.steps)
  for (const c of svc.consumers) visit(c.steps)
  return n
}

const ROW = 'grid grid-cols-[minmax(130px,1.2fr)_150px_54px_54px_minmax(130px,1fr)_34px] items-center gap-2 border-t border-line px-3 py-1.5'
const CHECK = 'flex cursor-pointer items-center justify-center gap-2 font-mono text-xs font-semibold text-muted'
const SIDE_NOTE = 'px-[18px] py-0.5 text-sm text-faint'

/**
 * The design of the tables of each service: a service owns its database, and what is designed here is
 * created when it starts. SQL steps read these tables, and are typed after their columns.
 */
export function TablesView() {
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  const [sel, setSel] = useState<{ service: string; table: string } | null>(null)

  const first = project.services.find((s) => s.tables.length > 0)
  const current = sel ?? (first ? { service: first.name, table: first.tables[0]!.name } : null)
  const si = project.services.findIndex((s) => s.name === current?.service)
  const service = project.services[si]
  const ti = service?.tables.findIndex((t) => t.name === current?.table) ?? -1
  const table = service?.tables[ti]

  const create = (serviceName: string) => {
    let name = ''
    edit((p) => {
      name = addTable(p.services.find((s) => s.name === serviceName)!).name
    })
    setSel({ service: serviceName, table: name })
  }

  return (
    <div data-testid="tables-view" className="absolute inset-0 grid grid-cols-[250px_minmax(0,1fr)] bg-bg">
      <nav className="overflow-y-auto border-r border-line bg-panel px-2 py-3" aria-label="Tables par service">
        {project.services.length === 0 && <p className={SIDE_NOTE}>Ajoutez d’abord un service dans l’architecture.</p>}
        {project.services.map((s) => (
          <section key={s.name} aria-label={`Tables de ${s.name}`}>
            <h3 className="mx-1.5 mt-0.5 mb-1 flex items-center justify-between text-xs tracking-[0.8px] text-muted uppercase">
              <span>{s.name}</span>
              <IconButton icon="plus" title={`Ajouter une table à ${s.name}`} size={13} onClick={() => create(s.name)} />
            </h3>
            {s.tables.length === 0 && <p className={SIDE_NOTE}>Aucune table</p>}
            {s.tables.map((t) => (
              <button
                key={t.name}
                type="button"
                className={`flex w-full cursor-pointer items-center gap-1.75 rounded-md border-0 py-1.25 pr-2.5 pl-4.5 text-left ${
                  current?.service === s.name && current.table === t.name ? 'bg-service/18 text-white' : 'bg-transparent text-text hover:bg-raised'
                }`}
                onClick={() => setSel({ service: s.name, table: t.name })}
              >
                <Icon name="table" size={13} /> <span className="mono">{t.name}</span>
              </button>
            ))}
          </section>
        ))}
      </nav>

      {service && table ? (
        <main className="grid min-w-0 auto-rows-max content-start gap-3.5 overflow-y-auto px-5.5 pt-4.5 pb-10" aria-label={`Table ${table.name}`}>
          <header className="flex items-center gap-3">
            <Icon name="table" size={18} />
            <span className="text-muted">{service.name} ·</span>
            <CommitInput
              value={table.name}
              mono
              label="Nom de la table"
              className="w-65 text-lg font-semibold"
              onCommit={(v) => {
                edit((p) => renameTable(p, service.name, table.name, v))
                setSel({ service: service.name, table: v })
              }}
            />
            <span className="flex-1" />
            <small className="text-muted">{usages(project, service.name, table.name)} étape(s) SQL l’utilisent</small>
            <IconButton
              icon="trash"
              title={`Supprimer la table ${table.name}`}
              danger
              onClick={() => {
                edit((p) => void p.services[si]!.tables.splice(ti, 1))
                setSel(null)
              }}
            />
          </header>

          <div className="overflow-hidden rounded-2xl border border-line bg-panel" role="table" aria-label="Colonnes">
            <div className={`${ROW} border-t-0 bg-panel-2 text-2xs tracking-[0.7px] text-faint uppercase`} role="row">
              <span>Nom</span>
              <span>Type</span>
              <span>Nul</span>
              <span>Clé</span>
              <span>Défaut (SQL)</span>
              <span />
            </div>
            {table.columns.map((c, ci) => (
              <div className={ROW} role="row" key={ci}>
                <CommitInput value={c.name} mono label="Nom de la colonne" onCommit={(v) => edit((p) => renameColumn(p, service.name, table.name, c.name, v))} />
                <select aria-label="Type de la colonne" value={c.type} onChange={(e) => edit((p) => (p.services[si]!.tables[ti]!.columns[ci]!.type = e.target.value as ColumnType))}>
                  {TYPES.map((t) => (
                    <option key={t} value={t}>{t} · {SQL_TYPE[t].toLowerCase()}</option>
                  ))}
                </select>
                <label className={CHECK} title="La colonne accepte NULL">
                  <input type="checkbox" aria-label={`${c.name} peut être nul`} checked={c.nullable} disabled={c.primaryKey} onChange={(e) => edit((p) => (p.services[si]!.tables[ti]!.columns[ci]!.nullable = e.target.checked))} />
                </label>
                <label className={CHECK} title="Clé primaire">
                  <input
                    type="checkbox"
                    aria-label={`${c.name} est une clé primaire`}
                    checked={c.primaryKey}
                    onChange={(e) =>
                      edit((p) => {
                        const col = p.services[si]!.tables[ti]!.columns[ci]!
                        col.primaryKey = e.target.checked
                        if (e.target.checked) col.nullable = false
                      })
                    }
                  />
                </label>
                <CommitInput allowEmpty value={c.default ?? ''} mono placeholder="now(), 0, 'x'…" label="Valeur par défaut" onCommit={(v) => edit((p) => (p.services[si]!.tables[ti]!.columns[ci]!.default = v))} />
                <IconButton icon="trash" title={`Supprimer la colonne ${c.name}`} danger onClick={() => edit((p) => void p.services[si]!.tables[ti]!.columns.splice(ci, 1))} />
              </div>
            ))}
          </div>
          <button type="button" className="link-btn" onClick={() => edit((p) => void addColumn(p.services[si]!.tables[ti]!))}>
            + Ajouter une colonne
          </button>
          <small className="text-muted">
            Les tables sont créées au démarrage du service. Ajouter une colonne met à jour une base existante ; renommer ou supprimer une colonne
            ne change pas les tables déjà créées. Une colonne ajoutée est nulle par défaut, pour ne pas échouer sur des lignes existantes.
          </small>

          <TableConstraints key={`${service.name}/${table.name}`} si={si} ti={ti} />

          <pre className="sql-preview mono" aria-label="SQL de la table">{createTableSql(table)}</pre>
        </main>
      ) : (
        <div className="grid h-full place-content-center justify-items-center gap-2.5 p-6 text-center text-faint">
          <Icon name="table" size={32} />
          <p>Chaque service a sa propre base. Créez une table pour concevoir ses colonnes ; les blocs SQL des routes pourront la lire.</p>
          {project.services.length > 0 && (
            <button type="button" className="link-btn" onClick={() => create(project.services[0]!.name)}>
              + Ajouter une table à {project.services[0]!.name}
            </button>
          )}
        </div>
      )}
    </div>
  )
}
