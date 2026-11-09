import { DEFAULT_REALTIME, renameService } from '../model'
import { useStore } from '../store'
import type { Project, RealtimeConfig, Service } from '../types'
import { CommitInput, NumberInput, Row, Section } from './ui'

/**
 * The inspector of a ready-made real-time service (SSE). The index of the open streams, the limits, the sessions, health and
 * metrics are fixed code; what is set here is five names. The rest (the address of DragonflyDB, the keys of the tokens, how many
 * streams one instance holds) is the environment of the deployment, never the project.
 */
export function RealtimeInspector({ index }: { index: number }) {
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  const select = useStore((s) => s.select)
  const service = project.services[index]!
  const svc = (p: Project): Service => p.services[index]!
  const config = service.realtime ?? DEFAULT_REALTIME
  const set = (field: keyof RealtimeConfig) => (value: string) =>
    edit((p) => {
      const s = svc(p)
      s.realtime = { ...(s.realtime ?? DEFAULT_REALTIME), [field]: value }
    })

  return (
    <div className="inspector">
      <Section title="Service temps réel" icon="service">
        <Row label="Nom">
          <CommitInput value={service.name} mono label="Nom du service" onCommit={(v) => { edit((p) => renameService(p, service.name, v)); select({ kind: 'service', name: v }) }} />
        </Row>
        <Row label="Port HTTP" hint="Le flux, la santé (/livez, /readyz) et les métriques (/metrics) sont sur ce port">
          <NumberInput value={service.httpPort ?? 0} min={1} label="Port HTTP" onChange={(v) => edit((p) => (svc(p).httpPort = v))} />
        </Row>
        <p className="muted small">
          Service prêt à l’emploi : l’API publie un événement sur un canal de DragonflyDB (ou Valkey, ou Redis) et le service le remet aux navigateurs qu’il concerne, en
          SSE. Un événement n’est cherché que parmi ses destinataires (par compte, et par entreprise et permission), pas parmi tous les flux ouverts : c’est ce qui permet de
          tenir des dizaines de milliers de flux sur une instance, dans une mémoire bornée.
        </p>
      </Section>

      <Section title="Contrat avec l’API" icon="publish">
        <Row label="Canal" hint="Où l’API publie : PUBLISH <canal> {“type”: “contact.updated”, “tenantId”: …, “permission”: “contacts.view”, “data”: {…}}">
          <CommitInput value={config.channel} mono label="Canal" onCommit={set('channel')} />
        </Row>
        <Row label="Chemin du flux" hint="Ce que le navigateur ouvre : GET <chemin>. Servi sur l’hôte de l’API derrière le proxy (TLS est son affaire)">
          <CommitInput value={config.path} mono label="Chemin du flux" onCommit={set('path')} />
        </Row>
        <Row label="Cookie du jeton" hint="Contient l’access JWT (EdDSA ; claims sub, sid, exp ; tid et permissions facultatifs). La clé publique est lue sur JWKS_URL">
          <CommitInput value={config.accessCookie} mono label="Cookie du jeton" onCommit={set('accessCookie')} />
        </Row>
        <Row label="Émetteur" hint="La valeur de iss que doivent porter les jetons">
          <CommitInput value={config.issuer} mono label="Émetteur" onCommit={set('issuer')} />
        </Row>
        <Row label="Préfixe des sessions" hint="L’API écrit <préfixe>:<session> = {accountId, tenantId (facultatif), exp} à chaque jeton émis. Un flux vit tant que sa session est renouvelée">
          <CommitInput value={config.sessionKeyPrefix} mono label="Préfixe des sessions" onCommit={set('sessionKeyPrefix')} />
        </Row>
        <p className="muted small">
          Dans l’environnement : <code>DRAGONFLY_URL</code>, <code>JWKS_URL</code>, <code>CORS_ALLOWED_ORIGINS</code>, et les limites{' '}
          <code>REALTIME_MAX_CONNECTIONS</code> (60000 : au-delà un flux est refusé en 503, la mémoire reste bornée), <code>REALTIME_MAX_PER_ACCOUNT</code>,{' '}
          <code>REALTIME_QUEUE</code>. <code>cargo run --release --example load</code> mesure le service sur votre matériel.
        </p>
      </Section>
    </div>
  )
}
