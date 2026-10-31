import type { ImageKind, Step } from '../types'
import { scalar, type Scope } from '../scope'
import { ExprEditor } from './ExprEditor'
import { NumberInput } from './ui'

type Update<T extends Step['type']> = (fn: (s: Extract<Step, { type: T }>) => void) => void
type Body<T extends Step['type']> = { step: Extract<Step, { type: T }>; update: Update<T>; scope: Scope }

const TEXT = scalar('string')
const KINDS: { kind: ImageKind; label: string }[] = [
  { kind: 'png', label: 'PNG' },
  { kind: 'jpeg', label: 'JPEG' },
]

function BucketEnv(props: { value: string; onChange: (v: string) => void }) {
  return (
    <div className="form-row">
      <span>Bucket</span>
      <input className="mono" aria-label="Variable d’environnement du bucket" value={props.value} spellCheck={false} onChange={(e) => props.onChange(e.target.value)} />
    </div>
  )
}

const STORE_NOTE = (
  <>
    Le nom du bucket est lu dans cette variable d’environnement, au démarrage. Avec <code>S3_ENDPOINT</code>, un stockage compatible (Garage, MinIO), adressé par chemin ; sans
    elle, AWS. Les identifiants sont ceux du SDK (<code>AWS_ACCESS_KEY_ID</code>, <code>AWS_SECRET_ACCESS_KEY</code>, <code>AWS_REGION</code>), jamais dans le projet. Un
    stockage injoignable est un 503.
  </>
)

/** `imageSniff`: the type of an image from its bytes. */
export function ImageSniffBody(props: Body<'imageSniff'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <ExprEditor label="Image (base64)" expr={step.input} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.input = v))} />
      <div className="form-row" role="group" aria-label="Types acceptés">
        <span>Types</span>
        {KINDS.map((k) => (
          <label className="check" key={k.kind}>
            <input
              type="checkbox"
              aria-label={`Accepter ${k.label}`}
              checked={step.allow.includes(k.kind)}
              onChange={(e) => u((s) => (s.allow = e.target.checked ? [...s.allow, k.kind] : s.allow.filter((x) => x !== k.kind)))}
            />{' '}
            {k.label}
          </label>
        ))}
      </div>
      <div className="form-row">
        <span>Taille max</span>
        <NumberInput label="Taille maximale de l’image (octets)" min={1} value={step.maxBytes} onChange={(n) => u((s) => (s.maxBytes = n))} />
        <small className="muted">octets, une fois décodée (2 Mo = 2097152)</small>
      </div>
      <small className="muted">
        Le client envoie l’image en base64 dans le JSON. Le type vient des <strong>premiers octets</strong> : le <code>Content-Type</code> du client n’est jamais cru, et il n’y a
        pas de SVG (il peut porter du script). Vide, trop grande, d’un autre type ou pas du base64 : 400. La limite de corps de la route est relevée pour que l’image passe.
      </small>
    </div>
  )
}

/** `objectPut`. */
export function ObjectPutBody(props: Body<'objectPut'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <BucketEnv value={step.bucketEnv} onChange={(v) => u((s) => (s.bucketEnv = v))} />
      <ExprEditor label="Clé de l’objet" expr={step.key} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.key = v))} />
      <ExprEditor label="Contenu (base64)" expr={step.body} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.body = v))} />
      <ExprEditor label="Type du contenu" expr={step.contentType} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.contentType = v))} />
      <small className="muted">
        {STORE_NOTE} Écrivez <strong>l’objet d’abord, la base ensuite</strong> : au pire un objet que la prochaine écriture remplace. Le type est celui que « Reconnaître une image » a trouvé.
      </small>
    </div>
  )
}

/** `objectGet`. */
export function ObjectGetBody(props: Body<'objectGet'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <BucketEnv value={step.bucketEnv} onChange={(v) => u((s) => (s.bucketEnv = v))} />
      <ExprEditor label="Clé de l’objet" expr={step.key} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.key = v))} />
      <small className="muted">La variable est le contenu en base64, ou rien si l’objet n’existe pas (ce n’est pas une erreur). {STORE_NOTE}</small>
    </div>
  )
}

/** `objectDelete`. */
export function ObjectDeleteBody(props: Body<'objectDelete'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <BucketEnv value={step.bucketEnv} onChange={(v) => u((s) => (s.bucketEnv = v))} />
      <ExprEditor label="Clé de l’objet" expr={step.key} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.key = v))} />
      <small className="muted">
        Supprimez <strong>la ligne de la base d’abord, l’objet ensuite</strong> : un objet que plus personne ne désigne est illisible, une ligne qui pointerait vers rien ne l’est
        pas. Supprimer un objet absent n’est pas une erreur. {STORE_NOTE}
      </small>
    </div>
  )
}
