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

const SIGNED_NOTE = (
  <>
    L’API ne fait que <strong>vérifier qui a le droit et signer</strong> : les octets passent directement entre le front et le stockage (ou un CDN devant lui), jamais par
    l’API. Si le front n’atteint pas le stockage à la même adresse que le service, <code>S3_PUBLIC_ENDPOINT</code> (une URL) donne celle pour laquelle l’URL est signée. {STORE_NOTE}
  </>
)

function Expiry(props: { value: number; onChange: (n: number) => void }) {
  return (
    <div className="form-row">
      <span>Durée</span>
      <NumberInput label="Durée de validité de l’URL (secondes)" min={1} value={props.value} onChange={props.onChange} />
      <small className="muted">secondes, de 1 à 604800 (7 jours)</small>
    </div>
  )
}

/** `objectPresignGet`. */
export function ObjectPresignGetBody(props: Body<'objectPresignGet'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <BucketEnv value={step.bucketEnv} onChange={(v) => u((s) => (s.bucketEnv = v))} />
      <ExprEditor label="Clé de l’objet" expr={step.key} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.key = v))} />
      <Expiry value={step.expiresSecs} onChange={(n) => u((s) => (s.expiresSecs = n))} />
      <small className="muted">
        La variable est l’URL à donner au front, qui lit l’objet directement. L’existence de l’objet n’est pas vérifiée (le stockage répondra 404 au front) : pour « donne-moi
        les 5 PDF », lisez les clés en base, signez-en une par objet et renvoyez les URL. {SIGNED_NOTE}
      </small>
    </div>
  )
}

/** `objectPresignPut`. */
export function ObjectPresignPutBody(props: Body<'objectPresignPut'>) {
  const { step, update: u, scope } = props
  return (
    <div className="stack">
      <BucketEnv value={step.bucketEnv} onChange={(v) => u((s) => (s.bucketEnv = v))} />
      <ExprEditor label="Clé de l’objet" expr={step.key} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.key = v))} />
      <ExprEditor label="Type du contenu" expr={step.contentType} target={TEXT} scope={scope} onChange={(v) => v && u((s) => (s.contentType = v))} />
      <ExprEditor label="Taille du fichier (octets)" expr={step.size} target={scalar('int64')} scope={scope} onChange={(v) => v && u((s) => (s.size = v))} />
      <div className="form-row">
        <span>Taille max</span>
        <NumberInput label="Taille maximale (octets)" min={1} value={step.maxBytes} onChange={(n) => u((s) => (s.maxBytes = n))} />
        <small className="muted">octets (2 Mo = 2097152, 10 Mio au plus) ; au-delà ou sous 1 : 400</small>
      </div>
      <Expiry value={step.expiresSecs} onChange={(n) => u((s) => (s.expiresSecs = n))} />
      <small className="muted">
        La variable est l’URL à donner au front, qui fait un <code>PUT</code> du fichier dessus. Le type et la taille sont <strong>signés</strong> : le front doit envoyer exactement
        ceux-là (en-têtes <code>Content-Type</code> et <code>Content-Length</code>), sinon le stockage refuse. Le type est la parole du client, il n’est pas reconnu d’après les
        octets comme avec « Reconnaître une image » : ne le servez pas tel quel dans une page. Enregistrez la ligne en base une fois l’envoi confirmé par le front. {SIGNED_NOTE}
      </small>
    </div>
  )
}
