import { type Scope, scalar } from '../scope'
import { useStore } from '../store'
import type { Expr, Step } from '../types'
import { ExprEditor } from './ExprEditor'
import { IconButton } from './ui'

type Body = { step: Extract<Step, { type: 'publishLive' }>; update: (fn: (s: Extract<Step, { type: 'publishLive' }>) => void) => void; scope: Scope }

const TEXT = scalar('string')
const ID = scalar('uuid')
const IDS = { ...scalar('uuid'), repeated: true }
/** What the real-time service takes in an event: a handful of identifiers. */
export const MAX_LIVE_DATA = 4
const EMPTY: Expr = { kind: 'literal', value: '' }

/** `publishLive`: an event for the browsers, sent through a real-time service once the flow has committed. */
export function PublishLiveBody({ step, update: u, scope }: Body) {
  const realtime = useStore((s) => s.project.services.filter((x) => x.kind === 'realtime'))
  const toAccount = step.accountId !== null
  return (
    <div className="stack">
      <label className="form-row">
        <span>Service temps réel</span>
        <select value={step.service} onChange={(e) => u((s) => (s.service = e.target.value))}>
          {!realtime.some((x) => x.name === step.service) && <option value={step.service}>{step.service || 'Choisir…'}</option>}
          {realtime.map((x) => (
            <option key={x.name}>{x.name}</option>
          ))}
        </select>
      </label>
      <label className="form-row">
        <span>Événement</span>
        <input className="mono" aria-label="Type de l’événement" value={step.event} spellCheck={false} placeholder="contact.updated" onChange={(e) => u((s) => (s.event = e.target.value))} />
      </label>
      <div className="form-row" role="group" aria-label="Destinataires">
        <span>Pour</span>
        <label className="check">
          <input
            type="radio"
            name={`audience-${step.id}`}
            checked={!toAccount}
            onChange={() =>
              u((s) => {
                s.accountId = null
                s.permission ??= 'module.view'
              })
            }
          />{' '}
          une permission ou des comptes
        </label>
        <label className="check">
          <input
            type="radio"
            name={`audience-${step.id}`}
            checked={toAccount}
            onChange={() =>
              u((s) => {
                s.accountId = EMPTY
                s.tenantId = null
                s.permission = null
                s.accountIds = null
              })
            }
          />{' '}
          un compte
        </label>
      </div>
      {toAccount ? (
        <ExprEditor label="Compte" expr={step.accountId} target={ID} scope={scope} onChange={(v) => v && u((s) => (s.accountId = v))} />
      ) : (
        <>
          <ExprEditor label="Entreprise" expr={step.tenantId} target={ID} scope={scope} allowUnset onChange={(v) => u((s) => (s.tenantId = v))} />
          <label className="form-row">
            <span>Permission</span>
            <input
              className="mono"
              aria-label="Permission requise"
              value={step.permission ?? ''}
              spellCheck={false}
              placeholder="contacts.view"
              onChange={(e) => u((s) => (s.permission = e.target.value === '' ? null : e.target.value))}
            />
          </label>
          <ExprEditor label="Comptes (en plus)" expr={step.accountIds} target={IDS} scope={scope} allowUnset onChange={(v) => u((s) => (s.accountIds = v))} />
        </>
      )}
      <ExprEditor label="Onglet d’origine" expr={step.origin} target={ID} scope={scope} allowUnset onChange={(v) => u((s) => (s.origin = v))} />
      {step.data.map((a, i) => (
        <div key={i} className="card" role="group" aria-label={`Donnée ${a.field}`}>
          <div className="form-row">
            <input className="mono" aria-label="Clé de la donnée" value={a.field} spellCheck={false} onChange={(e) => u((s) => (s.data[i]!.field = e.target.value))} />
            <IconButton icon="trash" title={`Supprimer la donnée ${a.field}`} danger onClick={() => u((s) => void s.data.splice(i, 1))} />
          </div>
          <ExprEditor label="Valeur" expr={a.value} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.data[i]!.value = v))} />
        </div>
      ))}
      {step.data.length < MAX_LIVE_DATA && (
        <button type="button" className="link-btn" onClick={() => u((s) => void s.data.push({ field: nextKey(s.data.map((d) => d.field)), value: EMPTY }))}>
          + Ajouter une donnée
        </button>
      )}
      <small className="muted">
        L’événement est envoyé <strong>après le commit</strong> du flux, jamais avant : un flux qui échoue n’envoie rien. Il n’est pas garanti (le canal ne garde rien) ; le
        navigateur relit son état à la réception, et sur <code>resync</code> après une coupure. Les destinataires sont toujours nommés : un compte, ou une permission
        (<code>module.view</code>, <code>module.manage</code>) et/ou des comptes, dans l’entreprise donnée (sans elle, les flux sans entreprise). Les données : {MAX_LIVE_DATA} entrées
        au plus, des identifiants de 64 caractères au plus, jamais du contenu. Le service lit le serveur dans <code>DRAGONFLY_URL</code>.
      </small>
    </div>
  )
}

function nextKey(taken: string[]): string {
  for (const base of ['id', 'ref']) if (!taken.includes(base)) return base
  let n = 1
  while (taken.includes(`key${n}`)) n++
  return `key${n}`
}
