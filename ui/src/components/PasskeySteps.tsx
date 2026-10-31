import type { Step } from '../types'
import { scalar, type Scope } from '../scope'
import { ExprEditor } from './ExprEditor'

type Update<T extends Step['type']> = (fn: (s: Extract<Step, { type: T }>) => void) => void
type Body<T extends Step['type']> = { step: Extract<Step, { type: T }>; update: Update<T>; scope: Scope }

const TEXT = scalar('string')

/** The answer of the browser is the JSON it produced, kept as a text by the client. */
const CREDENTIAL_HINT = 'Le JSON que produit le navigateur (navigator.credentials), transmis comme un texte.'

/** `passkeyRegisterStart`: the first request of registering a device. */
export function PasskeyRegisterStartBody(props: Body<'passkeyRegisterStart'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <ExprEditor label="Compte" expr={step.account} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.account = v))} />
      <ExprEditor label="Nom affiché par l’appareil" expr={step.name} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.name = v))} />
      <small className="muted">
        Le compte est déjà authentifié : prenez-le d’un jeton vérifié (<code>claims.sub</code>), jamais du corps de la requête. La variable reçoit{' '}
        <code>PasskeyChallenge</code> : <code>state_id</code>, à rendre à l’étape de fin, et <code>options</code>, le JSON de <code>navigator.credentials.create</code>. Un appareil
        déjà enregistré est exclu. Le domaine et l’origine viennent de la variable d’environnement <code>APP_URL</code> : le service ne démarre pas sans elle.
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
      <ExprEditor label="Compte" expr={step.account} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.account = v))} />
      <small className="muted">
        La variable reçoit <code>PasskeyChallenge</code> (<code>options</code> : le JSON de <code>navigator.credentials.get</code>). Un compte sans passkey est un{' '}
        <code>401 invalid passkey</code> : cela révèle qu’il n’en a pas, donc placez un « Limiter le débit » avant.
      </small>
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
      <small className="muted">
        {CREDENTIAL_HINT} La variable reçoit le compte pour lequel le défi a été émis : signez-lui un jeton. Le défi sert une fois ; faux, rejoué, expiré ou mal formé : le même{' '}
        <code>401 invalid passkey</code>. Le compteur de signature de la passkey est tenu à jour.
      </small>
    </div>
  )
}
