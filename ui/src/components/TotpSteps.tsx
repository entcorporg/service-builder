import type { Expr, Step } from '../types'
import { scalar, type Scope } from '../scope'
import { ExprEditor } from './ExprEditor'
import { NumberInput } from './ui'

type Update<T extends Step['type']> = (fn: (s: Extract<Step, { type: T }>) => void) => void

const TEXT = scalar('string')

function KeyEnv(props: { value: string; onChange: (v: string) => void }) {
  return (
    <div className="form-row">
      <span>Clé</span>
      <input className="mono" aria-label="Variable d’environnement de la clé" value={props.value} spellCheck={false} onChange={(e) => props.onChange(e.target.value)} />
    </div>
  )
}

/** The account a second factor belongs to: an id, not something that can change like an e-mail address. */
function Account(props: { expr: Expr; scope: Scope; onChange: (e: Expr) => void }) {
  return <ExprEditor label="Compte" expr={props.expr} target={TEXT} scope={props.scope} onChange={(v) => v && props.onChange(v)} />
}

/** `totpGenerate`: enrols an authenticator app. */
export function TotpGenerateBody(props: { step: Extract<Step, { type: 'totpGenerate' }>; update: Update<'totpGenerate'>; scope: Scope }) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <KeyEnv value={step.keyEnv} onChange={(v) => u((s) => (s.keyEnv = v))} />
      <div className="form-row">
        <span>Émetteur</span>
        <input className="mono" aria-label="Émetteur affiché par l’application" value={step.issuer} spellCheck={false} onChange={(e) => u((s) => (s.issuer = e.target.value))} />
      </div>
      <ExprEditor label="Libellé du compte" expr={step.account} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.account = v))} />
      <small className="muted">
        Le secret est <strong>chiffré</strong> (il faut pouvoir le relire pour vérifier un code), jamais haché. La variable reçoit <code>TotpSetup</code> : <code>sealed</code> à
        stocker, <code>otpauth_url</code> dont le client fait le QR code, et <code>secret</code> en base32 pour la saisie manuelle. Ces deux derniers ne sont pas conservés.
        Clé de 32 octets en 64 caractères hexadécimaux : le service ne démarre pas sans elle.
      </small>
    </div>
  )
}

/** `totpVerify`: a code against the sealed secret, once per time step. */
export function TotpVerifyBody(props: { step: Extract<Step, { type: 'totpVerify' }>; update: Update<'totpVerify'>; scope: Scope }) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <KeyEnv value={step.keyEnv} onChange={(v) => u((s) => (s.keyEnv = v))} />
      <ExprEditor label="Secret chiffré" expr={step.sealed} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.sealed = v))} />
      <ExprEditor label="Code saisi" expr={step.code} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.code = v))} />
      <Account expr={step.account} scope={scope} onChange={(v) => u((s) => (s.account = v))} />
      <small className="muted">
        Le code des 30 secondes précédentes, courantes ou suivantes est accepté ; chacun est comparé en temps constant. Un code ne sert <strong>qu’une fois</strong> : il doit
        correspondre à un pas plus récent que le dernier accepté pour ce compte (enregistré de façon atomique, deux requêtes simultanées ne passent pas toutes les deux). Faux,
        rejoué, mal formé ou compte sans 2FA : le même 401. Placez un « Limiter le débit » avant.
      </small>
    </div>
  )
}

/** `recoveryCodes`: a sheet of one-time codes. */
export function RecoveryCodesBody(props: { step: Extract<Step, { type: 'recoveryCodes' }>; update: Update<'recoveryCodes'>; scope: Scope }) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <Account expr={step.account} scope={scope} onChange={(v) => u((s) => (s.account = v))} />
      <div className="form-row">
        <span>Nombre</span>
        <NumberInput label="Nombre de codes de récupération" min={1} value={step.count} onChange={(n) => u((s) => (s.count = n))} />
      </div>
      <small className="muted">
        Des codes <code>XXXXX-XXXXX</code> (50 bits), à montrer une fois. Seules leurs empreintes, salées par compte, sont stockées ; en émettre de nouveaux efface les
        précédents. La variable est une liste de textes.
      </small>
    </div>
  )
}

/** `recoveryCodeUse`: spends a recovery code. */
export function RecoveryCodeUseBody(props: { step: Extract<Step, { type: 'recoveryCodeUse' }>; update: Update<'recoveryCodeUse'>; scope: Scope }) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <Account expr={step.account} scope={scope} onChange={(v) => u((s) => (s.account = v))} />
      <ExprEditor label="Code de récupération" expr={step.code} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.code = v))} />
      <small className="muted">Saisi avec ou sans tiret, en majuscules ou non. Valable une seule fois, même présenté deux fois en même temps. Sinon 401.</small>
    </div>
  )
}
