import { useCallback, useEffect, useState } from 'react'
import { api, openExternal } from '../bridge'
import { useStore } from '../store'
import type { AgentFeedback, McpInfo } from '../types'
import { shellQuote } from '../shell'
import { Icon } from './Icon'

function CopyBlock({ title, text }: { title: string; text: string }) {
  const notify = useStore((s) => s.notify)
  return (
    <div className="copy-block">
      <div className="copy-head">
        <strong>{title}</strong>
        <button
          type="button"
          className="ghost"
          aria-label={`Copier : ${title}`}
          onClick={() => void navigator.clipboard?.writeText(text).then(() => notify({ kind: 'info', text: 'Copié' }))}
        >
          <Icon name="copy" size={14} /> Copier
        </button>
      </div>
      <pre className="mono">{text}</pre>
    </div>
  )
}

const STATE_LABEL = { on: 'Actif', off: 'Inactif', stale: 'À mettre à jour' } as const

export function McpView() {
  const filePath = useStore((s) => s.filePath)
  const notify = useStore((s) => s.notify)
  const [info, setInfo] = useState<McpInfo | null>(null)
  const [busy, setBusy] = useState(false)
  const [feedback, setFeedback] = useState<AgentFeedback | null>(null)

  // Agents file reports while the app is in the background: read them again on every return.
  useEffect(() => {
    if (!filePath) {
      setFeedback(null)
      return
    }
    const load = () => void api.mcpFeedback(filePath).then(setFeedback).catch(() => setFeedback(null))
    load()
    window.addEventListener('focus', load)
    return () => window.removeEventListener('focus', load)
  }, [filePath])

  const reportAction = useCallback(
    async (run: () => Promise<AgentFeedback | void>) => {
      try {
        const next = await run()
        if (next) setFeedback(next)
      } catch (e) {
        notify({ kind: 'error', text: String(e) })
      }
    },
    [notify],
  )

  const openIssue = (id: number) =>
    reportAction(async () => {
      if (!filePath) return
      await openExternal(await api.mcpReportIssueUrl(filePath, id))
      return api.mcpFeedback(filePath)
    })

  useEffect(() => {
    let current = true
    api
      .mcpInfo(filePath)
      .then((i) => current && setInfo(i))
      .catch((e) => notify({ kind: 'error', text: String(e) }))
    return () => {
      current = false
    }
  }, [filePath, notify])

  const toggle = useCallback(
    async (enabled: boolean) => {
      if (!filePath) return
      setBusy(true)
      try {
        const activation = await api.mcpSetEnabled(filePath, enabled)
        setInfo((i) => i && { ...i, activation })
        notify({ kind: 'info', text: enabled ? `MCP activé dans ${activation.configPath}` : 'MCP désactivé' })
      } catch (e) {
        notify({ kind: 'error', text: String(e) })
      } finally {
        setBusy(false)
      }
    },
    [filePath, notify],
  )

  if (!info) return <div className="mcp-view" />

  const state = info.activation?.state
  const name = info.activation?.serverName ?? 'service-builder'
  const args = filePath ? ['mcp', filePath] : ['mcp', '<projet.sb.json>']
  const line = [info.command, ...args].map(shellQuote).join(' ')
  const json = JSON.stringify({ mcpServers: { [name]: { command: info.command, args } } }, null, 2)

  return (
    <div className="mcp-view">
      <div className="mcp-body">
        <header>
          <h2>Serveur MCP</h2>
          <p>
            Un client IA (Claude Code, Claude Desktop…) lance Service Builder en mode <span className="mono">mcp</span> et lit,
            modifie, valide et exporte ce projet par le protocole MCP, sur stdio. Le serveur travaille sur le fichier du projet.
          </p>
        </header>

        <section className="card" aria-label="Activation">
          <div className="activation">
            <div>
              <div className="activation-title">
                <strong>Activation</strong>
                {state && <span className={`pill ${state}`}>{STATE_LABEL[state]}</span>}
              </div>
              {info.activation ? (
                <small>
                  Ajoute l’entrée <span className="mono">{name}</span> au fichier <span className="mono">{info.activation.configPath}</span>,
                  que Claude Code lit depuis ce dossier. Les autres entrées sont conservées.
                </small>
              ) : (
                <small>Enregistrez le projet pour pouvoir activer le MCP : le serveur travaille sur son fichier.</small>
              )}
            </div>
            {info.activation &&
              (state === 'on' ? (
                <button type="button" className="ghost" disabled={busy} onClick={() => void toggle(false)}>
                  Désactiver
                </button>
              ) : (
                <button type="button" className="primary" disabled={busy} onClick={() => void toggle(true)}>
                  {state === 'stale' ? 'Mettre à jour' : 'Activer'}
                </button>
              ))}
          </div>
        </section>

        <section aria-label="Configuration manuelle">
          <h3>Configuration manuelle</h3>
          <CopyBlock title="Commande du serveur" text={line} />
          <CopyBlock title="Claude Code" text={`claude mcp add ${name} -- ${line}`} />
          <CopyBlock title="Autres clients (Claude Desktop…)" text={json} />
        </section>

        <section aria-label="Outils exposés">
          <h3>Outils exposés</h3>
          <ul className="tool-list">
            {info.tools.map((t) => (
              <li key={t.name}>
                <span className="mono tool-name">{t.name}</span>
                <span>{t.description}</span>
              </li>
            ))}
          </ul>
          <p className="hint">
            Les modifications d’un client sont écrites dans le fichier du projet. L’application le recharge quand la fenêtre
            reprend le focus, sauf si des modifications non enregistrées sont en cours.
          </p>
        </section>

        {feedback && (
          <section aria-label="Suggestions des agents">
            <h3>Suggestions des agents</h3>
            <div className="card feedback-head">
              <label className="check">
                <input
                  type="checkbox"
                  checked={feedback.enabled}
                  onChange={(e) => void reportAction(() => api.mcpFeedbackEnable(filePath!, e.target.checked))}
                />
                Demander un rapport à l’agent tous les 50 appels
              </label>
              <small>
                Un agent peut aussi en déposer un de lui-même. Rien n’est envoyé sans votre clic : « Envoyer sur GitHub » ouvre une
                issue pré-remplie dans votre navigateur, que vous relisez avant de la publier.
              </small>
            </div>
            {feedback.reports.length === 0 ? (
              <p className="hint">Aucun rapport pour l’instant.</p>
            ) : (
              feedback.reports.map((r) => (
                <article key={r.id} className="card report" aria-label={`Rapport ${r.id}`}>
                  <div className="report-head">
                    <small>{new Date(r.createdAt * 1000).toLocaleString('fr-FR')}</small>
                    {r.status === 'opened' && <span className="pill on">Ouvert sur GitHub</span>}
                  </div>
                  {r.summary && <p>{r.summary}</p>}
                  <ol>
                    {r.suggestions.map((s, i) => (
                      <li key={i}>
                        <strong>{s.title}</strong>
                        {s.details && <span>{s.details}</span>}
                      </li>
                    ))}
                  </ol>
                  <div className="report-actions">
                    <button type="button" className="primary" onClick={() => void openIssue(r.id)}>
                      Envoyer sur GitHub
                    </button>
                    <button type="button" className="ghost" onClick={() => void reportAction(() => api.mcpReportDismiss(filePath!, r.id))}>
                      Ignorer
                    </button>
                  </div>
                </article>
              ))
            )}
          </section>
        )}
      </div>
    </div>
  )
}
