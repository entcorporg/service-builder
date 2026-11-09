import type { Expr, Step } from '../types'
import { scalar, type Scope } from '../scope'
import { ExprEditor } from './ExprEditor'

type Update<T extends Step['type']> = (fn: (s: Extract<Step, { type: T }>) => void) => void
type Body<T extends Step['type']> = { step: Extract<Step, { type: T }>; update: Update<T>; scope: Scope }

const TEXT = scalar('string')

/** The answer of the browser is the JSON it produced, kept as a text by the client. */
const CREDENTIAL_HINT = 'Le JSON que produit le navigateur (navigator.credentials), transmis comme un texte.'

/**
 * The origin of the request, optional on the four steps: a service with several frontends (subdomains) names the one the
 * request comes from, for example from a header. Absent: `APP_URL`.
 */
function AppUrlField(props: { appUrl: Expr | null | undefined; onChange: (v: Expr | null) => void; scope: Scope; ending?: boolean }) {
  const { appUrl, onChange, scope, ending } = props
  return (
    <div className="stack">
      <label className="check" title="Plusieurs frontends : l’origine de la requête vient d’un champ ou d’un en-tête, au lieu de APP_URL">
        <input
          type="checkbox"
          aria-label="Origine propre à la requête"
          checked={!!appUrl}
          onChange={(e) => onChange(e.target.checked ? { kind: 'literal', value: '' } : null)}
        />{' '}
        origine propre à la requête (plusieurs frontends)
      </label>
      {appUrl && <ExprEditor label="Origine (https://app.exemple.fr)" expr={appUrl} target={TEXT} scope={scope} onChange={(v) => v && onChange(v)} />}
      {appUrl && (
        <small className="muted">
          {ending
            ? 'Doit être l’origine pour laquelle le défi a été émis (elle est gardée avec lui), sinon refus. Décochée, la fin prend celle du début.'
            : 'N’est acceptée que si c’est APP_URL ou si elle figure dans la variable d’environnement WEBAUTHN_ALLOWED_ORIGINS (origines séparées par des virgules) ; sinon 400. Elle est gardée avec le défi.'}{' '}
          Pour qu’une passkey serve sur plusieurs sous-domaines, fixez <code>WEBAUTHN_RP_ID</code> au domaine parent (<code>exemple.fr</code>) : toutes les origines doivent en dépendre.
        </small>
      )}
    </div>
  )
}

/** `passkeyRegisterStart`: the first request of registering a device. */
export function PasskeyRegisterStartBody(props: Body<'passkeyRegisterStart'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <ExprEditor label="Compte" expr={step.account} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.account = v))} />
      <ExprEditor label="Nom affiché par l’appareil" expr={step.name} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.name = v))} />
      <AppUrlField appUrl={step.appUrl} scope={scope} onChange={(v) => u((s) => (s.appUrl = v))} />
      <small className="muted">
        Le compte est déjà authentifié : prenez-le d’un jeton vérifié (<code>claims.sub</code>), jamais du corps de la requête. La variable reçoit{' '}
        <code>PasskeyChallenge</code> : <code>state_id</code>, à rendre à l’étape de fin, et <code>options</code>, le JSON de <code>navigator.credentials.create</code>. Un appareil
        déjà enregistré est exclu. Le domaine et l’origine viennent de la variable d’environnement <code>APP_URL</code> (sauf origine propre à la requête) : le service ne démarre pas sans elle.
      </small>
    </div>
  )
}

/** `passkeyRegisterFinish`: the second request. */
export function PasskeyRegisterFinishBody(props: Body<'passkeyRegisterFinish'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <ExprEditor label="Identifiant du défi" expr={step.stateId} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.stateId = v))} />
      <ExprEditor label="Réponse du navigateur" expr={step.credential} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.credential = v))} />
      <ExprEditor label="Compte" expr={step.account} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.account = v))} />
      <AppUrlField ending appUrl={step.appUrl} scope={scope} onChange={(v) => u((s) => (s.appUrl = v))} />
      <small className="muted">
        {CREDENTIAL_HINT} Le défi n’est valable que cinq minutes, une seule fois, et seulement pour le compte pour lequel il a été émis ; la clé d’un appareil ne peut être que
        sur un compte. Tout refus est un <code>400 invalid passkey</code>.
      </small>
    </div>
  )
}

/** `passkeyLoginStart`: the first request of a login. */
export function PasskeyLoginStartBody(props: Body<'passkeyLoginStart'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <label className="check" title="Aucun identifiant demandé : l’appareil propose les passkeys qu’il a pour le site">
        <input
          type="checkbox"
          aria-label="Sans identifiant"
          checked={!step.account}
          onChange={(e) => u((s) => (s.account = e.target.checked ? null : { kind: 'path', path: 'input.account' }))}
        />{' '}
        sans identifiant (passkey découvrable)
      </label>
      {step.account && <ExprEditor label="Compte" expr={step.account} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.account = v))} />}
      <AppUrlField appUrl={step.appUrl} scope={scope} onChange={(v) => u((s) => (s.appUrl = v))} />
      {step.account ? (
        <small className="muted">
          La variable reçoit <code>PasskeyChallenge</code> (<code>options</code> : le JSON de <code>navigator.credentials.get</code>). Un compte sans passkey est un{' '}
          <code>401 invalid passkey</code> : cela révèle qu’il n’en a pas, donc placez un « Limiter le débit » avant.
        </small>
      ) : (
        <small className="muted">
          Le défi ne nomme personne : le navigateur propose les passkeys qu’il détient pour le site (<code>navigator.credentials.get</code> avec <code>mediation: &quot;conditional&quot;</code>, ou en
          bouton) et l’étape de fin donne le compte à qui appartient la passkey signée. Seules les passkeys enregistrées par « début d’enregistrement » sont retrouvées ainsi, et seulement si
          l’appareil les garde de son côté. Chaque appel garde un défi cinq minutes, sans compte pour compter : placez un « Limiter le débit » avant, sur le client (un en-tête
          comme X-Forwarded-For, par exemple).
        </small>
      )}
    </div>
  )
}

/** `passkeyLoginFinish`: the second request; it binds the account that logged in. */
export function PasskeyLoginFinishBody(props: Body<'passkeyLoginFinish'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <ExprEditor label="Identifiant du défi" expr={step.stateId} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.stateId = v))} />
      <ExprEditor label="Réponse du navigateur" expr={step.credential} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.credential = v))} />
      <AppUrlField ending appUrl={step.appUrl} scope={scope} onChange={(v) => u((s) => (s.appUrl = v))} />
      <small className="muted">
        {CREDENTIAL_HINT} La variable reçoit le compte pour lequel le défi a été émis : signez-lui un jeton. Le défi sert une fois ; faux, rejoué, expiré ou mal formé : le même{' '}
        <code>401 invalid passkey</code>. Le compteur de signature de la passkey est tenu à jour.
      </small>
    </div>
  )
}
