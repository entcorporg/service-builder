import { useStore } from '../store'
import type { Assignment, JwksSource, Step } from '../types'
import { scalar, type Scope } from '../scope'
import { ExprEditor } from './ExprEditor'
import { IconButton, NumberInput } from './ui'

type Update<T extends Step['type']> = (fn: (s: Extract<Step, { type: T }>) => void) => void

const TEXT = scalar('string')

/** A list of environment variable names, one per line of seed: the first signs, the others stay published. */
function EnvList(props: { label: string; names: string[]; onChange: (names: string[]) => void }) {
  return (
    <div className="stack" role="group" aria-label={props.label}>
      {props.names.map((name, i) => (
        <div className="form-row" key={i}>
          <input className="mono" aria-label={`${props.label} ${i + 1}`} value={name} spellCheck={false} onChange={(e) => props.onChange(props.names.map((n, j) => (j === i ? e.target.value : n)))} />
          <IconButton icon="trash" title={`Retirer ${name || 'cette graine'}`} danger disabled={props.names.length <= 1} onClick={() => props.onChange(props.names.filter((_, j) => j !== i))} />
        </div>
      ))}
      <button type="button" className="link-btn" onClick={() => props.onChange([...props.names, ''])}>
        + Ajouter une graine
      </button>
    </div>
  )
}

/** `signJwt`: the claims of a token, each a name and a value of the flow. */
export function SignJwtBody(props: { step: Extract<Step, { type: 'signJwt' }>; update: Update<'signJwt'>; scope: Scope }) {
  const { step, update: u, scope } = props
  const setClaim = (i: number, fn: (c: Assignment) => void) => u((s) => fn(s.claims[i]!))
  return (
    <div className="stack">
      <div className="form-row">
        <span>Graine</span>
        <input className="mono" aria-label="Variable d’environnement de la graine" value={step.keyEnv} spellCheck={false} onChange={(e) => u((s) => (s.keyEnv = e.target.value))} />
      </div>
      <div className="form-row">
        <span>Émetteur</span>
        <input className="mono" aria-label="Émetteur du jeton" value={step.issuer} spellCheck={false} onChange={(e) => u((s) => (s.issuer = e.target.value))} />
        <span>Durée</span>
        <NumberInput label="Durée de vie du jeton (secondes)" min={1} value={step.ttlSecs} onChange={(n) => u((s) => (s.ttlSecs = n))} />
        <small className="muted">s</small>
      </div>
      <h5 className="mini-title">Claims</h5>
      {step.claims.map((c, i) => (
        <div className="stack" key={i} role="group" aria-label={`Claim ${c.field}`}>
          <div className="form-row">
            <input className="mono" aria-label="Nom du claim" value={c.field} spellCheck={false} onChange={(e) => setClaim(i, (x) => (x.field = e.target.value))} />
            <IconButton icon="trash" title={`Retirer le claim ${c.field}`} danger onClick={() => u((s) => void s.claims.splice(i, 1))} />
          </div>
          <ExprEditor label={`Valeur du claim ${c.field}`} expr={c.value} target={null} scope={scope} onChange={(v) => v && setClaim(i, (x) => (x.value = v))} />
        </div>
      ))}
      <button type="button" className="link-btn" onClick={() => u((s) => void s.claims.push({ field: s.claims.some((c) => c.field === 'roles') ? 'tid' : 'roles', value: { kind: 'literal', value: '' } }))}>
        + Ajouter un claim
      </button>
      <small className="muted">
        Un claim est du texte, un nombre, un booléen ou une liste de ceux-ci (« Rust » pour une liste). <code>sub</code> est obligatoire ; <code>iss</code>,{' '}
        <code>iat</code> et <code>exp</code> sont posés par l’étape. Algorithme EdDSA (Ed25519), graine de 32 octets en 64 caractères hexadécimaux : le
        service ne démarre pas sans elle.
      </small>
    </div>
  )
}

/** `verifyJwt`: the token, where the keys come from, and the schema the claims are read as. */
export function VerifyJwtBody(props: { step: Extract<Step, { type: 'verifyJwt' }>; update: Update<'verifyJwt'>; scope: Scope }) {
  const { step, update: u, scope } = props
  const schemas = useStore((s) => s.project.messages).filter((m) => m.kind === 'api')
  const setSource = (source: JwksSource) => u((s) => (s.source = source))
  return (
    <div className="stack">
      <ExprEditor label="Jeton" expr={step.token} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.token = v))} />
      <label className="check" title="Le jeton est la valeur d’un en-tête « Authorization: Bearer … »">
        <input type="checkbox" aria-label="En-tête Authorization Bearer" checked={step.bearer} onChange={(e) => u((s) => (s.bearer = e.target.checked))} /> c’est un en-tête <code>Authorization: Bearer</code>
      </label>
      <div className="form-row">
        <span>Clés</span>
        <select aria-label="Origine des clés" value={step.source.kind} onChange={(e) => setSource(e.target.value === 'url' ? { kind: 'url', urlEnv: 'AUTH_JWKS_URL' } : { kind: 'keys', keyEnvs: ['JWT_SIGNING_KEY'] })}>
          <option value="keys">les graines de ce service</option>
          <option value="url">le JWKS du service qui signe</option>
        </select>
      </div>
      {step.source.kind === 'keys' ? (
        <EnvList label="Graine" names={step.source.keyEnvs} onChange={(keyEnvs) => setSource({ kind: 'keys', keyEnvs })} />
      ) : (
        <div className="form-row">
          <span>URL dans</span>
          <input className="mono" aria-label="Variable d’environnement de l’URL du JWKS" value={step.source.urlEnv} spellCheck={false} onChange={(e) => setSource({ kind: 'url', urlEnv: e.target.value })} />
        </div>
      )}
      <div className="form-row">
        <span>Émetteur</span>
        <input className="mono" aria-label="Émetteur attendu" value={step.issuer} spellCheck={false} onChange={(e) => u((s) => (s.issuer = e.target.value))} />
      </div>
      <div className="form-row">
        <span>Claims lus comme</span>
        <select aria-label="Schéma des claims" value={step.schema} onChange={(e) => u((s) => (s.schema = e.target.value))}>
          {!schemas.some((m) => m.name === step.schema) && <option value={step.schema}>{step.schema || 'Choisir…'}</option>}
          {schemas.map((m) => (
            <option key={m.name}>{m.name}</option>
          ))}
        </select>
        <input className="mono" aria-label="Nom de la variable" value={step.bind} spellCheck={false} onChange={(e) => u((s) => (s.bind = e.target.value))} />
      </div>
      <small className="muted">
        Algorithme EdDSA seulement (ni <code>none</code>, ni HS256), émetteur, expiration (5 s de tolérance) et <code>sub</code> vérifiés ; la clé est choisie par le{' '}
        <code>kid</code> du jeton, donc une rotation ne coupe pas les jetons en cours. Tout refus est le même <code>401 invalid token</code>. Un claim absent a la
        valeur par défaut de son champ. Avec le JWKS d’un autre service, l’URL (<code>http://…</code>) est lue une fois, puis de nouveau si le jeton nomme une clé inconnue.
      </small>
    </div>
  )
}

/** `jwks`: the seeds whose public keys are published. */
export function JwksBody(props: { step: Extract<Step, { type: 'jwks' }>; update: Update<'jwks'> }) {
  const { step, update: u } = props
  return (
    <div className="stack">
      <EnvList label="Graine publiée" names={step.keyEnvs} onChange={(keyEnvs) => u((s) => (s.keyEnvs = keyEnvs))} />
      <div className="form-row">
        <span>Résultat</span>
        <input className="mono" aria-label="Nom de la variable" value={step.bind} spellCheck={false} onChange={(e) => u((s) => (s.bind = e.target.value))} />
        <small className="muted">de type <code>Jwks</code></small>
      </div>
      <small className="muted">
        Les clés publiques (<code>kty</code>, <code>crv</code>, <code>alg</code>, <code>kid</code>, <code>x</code>), jamais la graine. La première graine signe : en
        rotation, mettez la nouvelle en premier et gardez l’ancienne tant que ses jetons n’ont pas expiré. Les schémas <code>Jwks</code> et <code>Jwk</code> sont créés au besoin ;
        servez la variable depuis une route publique, par exemple <code>GET /.well-known/jwks.json</code>.
      </small>
    </div>
  )
}
