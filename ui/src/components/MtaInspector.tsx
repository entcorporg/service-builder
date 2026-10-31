import { addMailEvent, mailFields, removeMailEvent, renameService, setMailTopic } from '../model'
import { useStore } from '../store'
import type { DedupFallback, MailRule, MailTemplate, Project, Service } from '../types'
import { Collapsible, CommitInput, Empty, IconButton, NumberInput, Row, Section } from './ui'

const FALLBACKS: { value: DedupFallback; label: string; hint: string }[] = [
  { value: 'topicAndRecipient', label: 'un mail par sujet et adresse', hint: 'Sans identité, la clé est « topic:adresse »' },
  { value: 'recipient', label: 'un mail par adresse', hint: 'Sans identité, un seul mail par adresse' },
  { value: 'reject', label: 'refuser l’événement', hint: 'Jamais de mail sans identité d’événement : l’événement part sur le .dlq' },
]

/** A select of the text fields of the event; `none` adds an empty choice (an optional field). */
function FieldSelect(props: { label: string; fields: string[]; value: string | undefined; none?: string; onChange: (v: string | undefined) => void }) {
  const known = props.value === undefined || props.fields.includes(props.value)
  return (
    <select aria-label={props.label} value={props.value ?? ''} onChange={(e) => props.onChange(e.target.value === '' ? undefined : e.target.value)}>
      {props.none !== undefined && <option value="">{props.none}</option>}
      {!known && <option value={props.value}>{props.value} (absent de l’événement)</option>}
      {props.value === '' && props.none === undefined && <option value="">Choisir…</option>}
      {props.fields.map((f) => (
        <option key={f} value={f}>{f}</option>
      ))}
    </select>
  )
}

/**
 * The inspector of a ready-made mail service: its events and their mails. The queue, the worker, DKIM, SMTP and the bounces
 * are fixed code; what is drawn here is which events are mailed, who they go to, and what the mail says.
 */
export function MtaInspector({ index }: { index: number }) {
  const project = useStore((s) => s.project)
  const edit = useStore((s) => s.edit)
  const select = useStore((s) => s.select)
  const service = project.services[index]!
  const svc = (p: Project): Service => p.services[index]!
  const free = project.topics.filter((t) => !service.consumers.some((c) => c.topic === t.name))

  return (
    <div className="inspector">
      <Section title="Service mail" icon="service">
        <Row label="Nom">
          <CommitInput value={service.name} mono label="Nom du service" onCommit={(v) => { edit((p) => renameService(p, service.name, v)); select({ kind: 'service', name: v }) }} />
        </Row>
        <Row label="Port HTTP" hint="Santé (/health, /livez, /readyz) et métriques (/metrics)">
          <NumberInput value={service.httpPort ?? 0} min={1} label="Port HTTP" onChange={(v) => edit((p) => (svc(p).httpPort = v))} />
        </Row>
        <p className="muted small">
          Service prêt à l’emploi : file d’envoi, worker, signature DKIM, livraison SMTP, réception des rebonds. Il n’y a rien d’autre à dessiner que les événements
          qu’il envoie et leurs mails. Les secrets (clé DKIM, relais SMTP) sont dans l’environnement, jamais dans le projet.
        </p>
      </Section>

      <Section
        title="Événements et mails"
        icon="publish"
        action={<IconButton icon="plus" title={free.length ? 'Ajouter un mail' : 'Aucun topic sans mail : créez d’abord un topic'} disabled={free.length === 0} onClick={() => edit((p) => void addMailEvent(p, svc(p)))} />}
      >
        {service.consumers.length === 0 && <Empty>Aucun événement : ce service n’envoie aucun mail. Ajoutez un mail, ou tirez une flèche d’un topic vers lui.</Empty>}
        {service.consumers.map((c, ci) => {
          const rule = c.mail
          const template = service.templates?.find((t) => t.eventType === c.topic)
          const fields = mailFields(project, c.topic)
          const setRule = (fn: (r: MailRule) => void) => edit((p) => fn(svc(p).consumers[ci]!.mail!))
          const setTemplate = (fn: (t: MailTemplate) => void) =>
            edit((p) => {
              const s = svc(p)
              let t = s.templates?.find((x) => x.eventType === c.topic)
              if (!t) s.templates = [...(s.templates ?? []), (t = { eventType: c.topic, subject: '', html: '', text: '' })]
              fn(t)
            })
          return (
            <Collapsible
              key={ci}
              tone="consumer"
              defaultOpen={service.consumers.length === 1}
              title={<><strong className="mono">{c.topic}</strong><span className="step-summary">{template?.subject || 'sans sujet'}</span></>}
              actions={<IconButton icon="trash" title={`Supprimer le mail de ${c.topic}`} danger onClick={() => edit((p) => removeMailEvent(svc(p), ci))} />}
            >
              <Row label="Événement">
                <select aria-label="Événement envoyé" value={c.topic} onChange={(e) => edit((p) => setMailTopic(p, svc(p), ci, e.target.value))}>
                  <option value={c.topic}>{c.topic}</option>
                  {free.map((t) => (
                    <option key={t.name} value={t.name}>{t.name}</option>
                  ))}
                </select>
              </Row>
              {rule && (
                <>
                  <Row label="Adresse" hint="Le champ de l’événement qui porte l’adresse du destinataire">
                    <FieldSelect label="Champ de l’adresse" fields={fields} value={rule.recipient} onChange={(v) => v && setRule((r) => (r.recipient = v))} />
                  </Row>
                  <Row label="Adresse facultative" hint="Sans adresse, l’événement est ignoré au lieu d’être une erreur">
                    <label className="check">
                      <input type="checkbox" aria-label="Adresse facultative" checked={rule.optionalRecipient} onChange={(e) => setRule((r) => (r.optionalRecipient = e.target.checked))} /> ignorer sans adresse
                    </label>
                  </Row>
                  <Row label="Identité" hint="Le même événement redélivré par Kafka ne part qu’une fois">
                    <FieldSelect label="Champ d’identité" fields={fields} value={rule.dedupField} onChange={(v) => v && setRule((r) => (r.dedupField = v))} />
                  </Row>
                  <Row label="Sans identité">
                    <select aria-label="Sans identité" value={rule.dedupFallback} title={FALLBACKS.find((f) => f.value === rule.dedupFallback)?.hint} onChange={(e) => setRule((r) => (r.dedupFallback = e.target.value as DedupFallback))}>
                      {FALLBACKS.map((f) => (
                        <option key={f.value} value={f.value}>{f.label}</option>
                      ))}
                    </select>
                  </Row>
                  <Row label="Entreprise" hint="Un champ avec un nom d’entreprise : l’expéditeur s’affiche « entreprise via <nom du service> »">
                    <FieldSelect label="Champ de l’entreprise" fields={fields} value={rule.senderCompanyVia} none="aucune" onChange={(v) => setRule((r) => (r.senderCompanyVia = v))} />
                  </Row>
                  <Row label="Répondre à" hint="Un champ avec l’adresse où vont les réponses">
                    <FieldSelect label="Champ de l’adresse de réponse" fields={fields} value={rule.replyTo} none="expéditeur" onChange={(v) => setRule((r) => (r.replyTo = v))} />
                  </Row>
                </>
              )}
              <h4 className="steps-title">Le mail</h4>
              <Row label="Sujet">
                <input aria-label="Sujet du mail" value={template?.subject ?? ''} onChange={(e) => setTemplate((t) => (t.subject = e.target.value))} />
              </Row>
              <label className="field-label">HTML</label>
              <textarea className="mono code-input" aria-label="Corps HTML du mail" rows={7} spellCheck={false} value={template?.html ?? ''} onChange={(e) => setTemplate((t) => (t.html = e.target.value))} />
              <label className="field-label">Texte</label>
              <textarea className="mono code-input" aria-label="Corps texte du mail" rows={5} spellCheck={false} value={template?.text ?? ''} onChange={(e) => setTemplate((t) => (t.text = e.target.value))} />
              <p className="muted small">
                Modèle <a href="https://keats.github.io/tera/docs/" target="_blank" rel="noreferrer">Tera</a> rendu avec les champs de l’événement :{' '}
                {fields.length ? fields.map((f) => <code key={f}>{`{{ ${f} }}`} </code>) : 'aucun champ texte'}. Le HTML est échappé ; le sujet et le texte ne le sont pas. Ajoutez{' '}
                <code>| safe</code> à un lien du HTML pour qu’il ne soit pas échappé. Une fois le service lancé, le mail se modifie aussi dans la table <code>mail_templates</code>, sans redémarrage.
              </p>
            </Collapsible>
          )
        })}
      </Section>
    </div>
  )
}
