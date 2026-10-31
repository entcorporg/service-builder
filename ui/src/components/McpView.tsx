import { useCallback, useEffect, useState } from 'react'
import { api, openExternal } from '../bridge'
import { useStore } from '../store'
import type { AgentFeedback, McpInfo } from '../types'
import { shellQuote } from '../shell'
import { Icon } from './Icon'

function CopyBlock({ title, text }: { title: string; text: string }) {
  const notify = useStore((s) => s.notify)
  return (
    <div className="mb-2.5 overflow-hidden rounded-xl border border-line bg-panel">
      <div className="flex items-center justify-between border-b border-line py-1 pr-1.5 pl-3">
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
      <pre data-testid="copy-text" className="m-0 overflow-x-auto px-3 py-2.5 font-mono text-sm whitespace-pre-wrap wrap-anywhere">{text}</pre>
    </div>
  )
}

const CARD = 'min-w-0 rounded-xl border border-line bg-panel-2 px-4 py-[14px]'
const H3 = 'mt-1.5 mb-2 text-xs tracking-[0.8px] text-muted uppercase'
const SMALL = 'mt-1 block leading-[1.5] text-muted'
const PILL = 'rounded-full px-[9px] py-0.5 text-xs font-bold'
const PILL_TONE = { on: 'bg-ok/12 text-ok', stale: 'bg-warn/12 text-warn', off: 'bg-raised text-muted' } as const

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

  if (!info) return <div data-testid="mcp-view" className="absolute inset-0 overflow-y-auto bg-bg" />

  const state = info.activation?.state
  const name = info.activation?.serverName ?? 'service-builder'
  const args = filePath ? ['mcp', filePath] : ['mcp', '<projet.sb.json>']
  const line = [info.command, ...args].map(shellQuote).join(' ')
  const json = JSON.stringify({ mcpServers: { [name]: { command: info.command, args } } }, null, 2)

  return (
    <div data-testid="mcp-view" className="absolute inset-0 overflow-y-auto bg-bg">
      <div className="mx-auto grid max-w-215 gap-4 px-6 pt-5.5 pb-12">
        <header>
          <h2 className="mb-1 text-xl">Serveur MCP</h2>
          <p className="leading-normal text-muted">
            Un client IA (Claude Code, Claude Desktop…) lance Service Builder en mode <span className="font-mono text-sm">mcp</span> et lit,
            modifie, valide et exporte ce projet par le protocole MCP, sur stdio. Le serveur travaille sur le fichier du projet.
          </p>
        </header>

        <section className={CARD} aria-label="Activation">
          <div className="flex items-center justify-between gap-4.5">
            <div>
              <div className="flex items-center gap-2.5">
                <strong>Activation</strong>
                {state && <span data-testid="mcp-state" data-state={state} className={`${PILL} ${PILL_TONE[state]}`}>{STATE_LABEL[state]}</span>}
              </div>
              {info.activation ? (
                <small className={SMALL}>
                  Ajoute l’entrée <span className="font-mono text-sm">{name}</span> au fichier <span className="font-mono text-sm">{info.activation.configPath}</span>,
                  que Claude Code lit depuis ce dossier. Les autres entrées sont conservées.
                </small>
              ) : (
                <small className={SMALL}>Enregistrez le projet pour pouvoir activer le MCP : le serveur travaille sur son fichier.</small>
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
          <h3 className={H3}>Configuration manuelle</h3>
          <CopyBlock title="Commande du serveur" text={line} />
          <CopyBlock title="Claude Code" text={`claude mcp add ${name} -- ${line}`} />
          <CopyBlock title="Autres clients (Claude Desktop…)" text={json} />
        </section>

        <section aria-label="Outils exposés">
          <h3 className={H3}>Outils exposés</h3>
          <ul aria-label="Liste des outils" className="mx-0 mt-0 mb-3 list-none rounded-xl border border-line bg-panel p-0">
            {info.tools.map((t) => (
              <li key={t.name} className="grid grid-cols-[190px_minmax(0,1fr)] gap-3 border-t border-line px-3 py-2 leading-[1.45] text-muted first:border-t-0">
                <span className="font-mono text-sm text-text">{t.name}</span>
                <span>{t.description}</span>
              </li>
            ))}
          </ul>
          <p className="text-sm leading-normal text-muted">
            Les modifications d’un client sont écrites dans le fichier du projet. L’application le recharge quand la fenêtre
            reprend le focus, sauf si des modifications non enregistrées sont en cours.
          </p>
        </section>

        {feedback && (
          <section aria-label="Suggestions des agents">
            <h3 className={H3}>Suggestions des agents</h3>
            <div className={`${CARD} grid gap-1.5`}>
              <label className="flex cursor-pointer items-center gap-2 font-mono text-xs font-semibold text-muted">
                <input
                  type="checkbox"
                  checked={feedback.enabled}
                  onChange={(e) => void reportAction(() => api.mcpFeedbackEnable(filePath!, e.target.checked))}
                />
                Demander un rapport à l’agent tous les 50 appels
              </label>
              <small className="leading-normal text-muted">
                Un agent peut aussi en déposer un de lui-même. Rien n’est envoyé sans votre clic : « Envoyer sur GitHub » ouvre une
                issue pré-remplie dans votre navigateur, que vous relisez avant de la publier.
              </small>
            </div>
            {feedback.reports.length === 0 ? (
              <p className="text-sm leading-normal text-muted">Aucun rapport pour l’instant.</p>
            ) : (
              feedback.reports.map((r) => (
                <article key={r.id} className={`${CARD} grid gap-2`} aria-label={`Rapport ${r.id}`}>
                  <div className="flex items-center gap-2.5 text-muted">
                    <small>{new Date(r.createdAt * 1000).toLocaleString('fr-FR')}</small>
                    {r.status === 'opened' && <span className={`${PILL} ${PILL_TONE.on}`}>Ouvert sur GitHub</span>}
                  </div>
                  {r.summary && <p className="leading-normal text-muted">{r.summary}</p>}
                  <ol className="m-0 grid gap-1.5 pl-5">
                    {r.suggestions.map((s, i) => (
                      <li key={i}>
                        <strong>{s.title}</strong>
                        {s.details && <span className="mt-0.5 block leading-[1.45] text-muted">{s.details}</span>}
                      </li>
                    ))}
                  </ol>
                  <div className="flex gap-2">
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
