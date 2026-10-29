import { useState } from 'react'
import { addColumn, addTable, renameColumn, renameTable } from '../model'
import { useStore } from '../store'
import type { ColumnType, Project, Table } from '../types'
import { Icon } from './Icon'
import { CommitInput, Empty, IconButton } from './ui'

const TYPES: ColumnType[] = ['string', 'bool', 'int32', 'int64', 'float', 'double', 'timestamp', 'uuid']
const SQL_TYPE: Record<ColumnType, string> = {
  string: 'TEXT',
  bool: 'BOOLEAN',
  int32: 'INTEGER',
  int64: 'BIGINT',
  float: 'REAL',
  double: 'DOUBLE PRECISION',
  timestamp: 'TIMESTAMPTZ',
  uuid: 'UUID',
}

/** The `CREATE TABLE` the service applies at start-up, to read the design as SQL. */
export function createTableSql(t: Table): string {
  const cols = t.columns.map((c) => `  ${c.name} ${SQL_TYPE[c.type]}${c.nullable ? '' : ' NOT NULL'}${c.default?.trim() ? ` DEFAULT ${c.default.trim()}` : ''}`)
  const pk = t.columns.filter((c) => c.primaryKey).map((c) => c.name)
  if (pk.length) cols.push(`  PRIMARY KEY (${pk.join(', ')})`)
  return `CREATE TABLE IF NOT EXISTS ${t.name} (\n${cols.join(',\n')}\n);`
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
    <div className="tables-view">
      <nav className="tv-side" aria-label="Tables par service">
        {project.services.length === 0 && <Empty>Ajoutez d’abord un service dans l’architecture.</Empty>}
        {project.services.map((s) => (
          <section key={s.name} aria-label={`Tables de ${s.name}`}>
            <h3>
              <span>{s.name}</span>
              <IconButton icon="plus" title={`Ajouter une table à ${s.name}`} size={13} onClick={() => create(s.name)} />
            </h3>
            {s.tables.length === 0 && <p className="empty">Aucune table</p>}
            {s.tables.map((t) => (
              <button
                key={t.name}
                type="button"
                className={`tv-table${current?.service === s.name && current.table === t.name ? ' active' : ''}`}
                onClick={() => setSel({ service: s.name, table: t.name })}
              >
                <Icon name="table" size={13} /> <span className="mono">{t.name}</span>
              </button>
            ))}
          </section>
        ))}
      </nav>

      {service && table ? (
        <main className="tv-main" aria-label={`Table ${table.name}`}>
          <header className="tv-head">
            <Icon name="table" size={18} />
            <span className="tv-service">{service.name} ·</span>
            <CommitInput
              value={table.name}
              mono
              label="Nom de la table"
              onCommit={(v) => {
                edit((p) => renameTable(p, service.name, table.name, v))
                setSel({ service: service.name, table: v })
              }}
            />
            <span className="spacer" />
            <small className="muted">{usages(project, service.name, table.name)} étape(s) SQL l’utilisent</small>
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

          <div className="tv-columns" role="table" aria-label="Colonnes">
            <div className="tv-row head" role="row">
              <span>Nom</span>
              <span>Type</span>
              <span>Nul</span>
              <span>Clé</span>
              <span>Défaut (SQL)</span>
              <span />
            </div>
            {table.columns.map((c, ci) => (
              <div className="tv-row" role="row" key={ci}>
                <CommitInput value={c.name} mono label="Nom de la colonne" onCommit={(v) => edit((p) => renameColumn(p, service.name, table.name, c.name, v))} />
                <select aria-label="Type de la colonne" value={c.type} onChange={(e) => edit((p) => (p.services[si]!.tables[ti]!.columns[ci]!.type = e.target.value as ColumnType))}>
                  {TYPES.map((t) => (
                    <option key={t} value={t}>{t} · {SQL_TYPE[t].toLowerCase()}</option>
                  ))}
                </select>
                <label className="check" title="La colonne accepte NULL">
                  <input type="checkbox" aria-label={`${c.name} peut être nul`} checked={c.nullable} disabled={c.primaryKey} onChange={(e) => edit((p) => (p.services[si]!.tables[ti]!.columns[ci]!.nullable = e.target.checked))} />
                </label>
                <label className="check" title="Clé primaire">
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
          <small className="muted">
            Les tables sont créées au démarrage du service. Ajouter une colonne met à jour une base existante ; renommer ou supprimer une colonne
            ne change pas les tables déjà créées. Une colonne ajoutée est nulle par défaut, pour ne pas échouer sur des lignes existantes.
          </small>

          <pre className="sql-preview tv-sql mono" aria-label="SQL de la table">{createTableSql(table)}</pre>
        </main>
      ) : (
        <div className="tv-empty">
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
