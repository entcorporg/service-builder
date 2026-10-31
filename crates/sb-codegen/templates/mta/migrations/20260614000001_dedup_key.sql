-- Idempotence du consumer Kafka (at-least-once) : un event rejoué ne doit pas
-- générer un mail en double. `dedup_key` porte une clé naturelle par event
-- (user_id pour la bienvenue, event_id pour les mails sécurité) ; l'INSERT du
-- consumer fait `ON CONFLICT (dedup_key) DO NOTHING`.
ALTER TABLE mail_queue ADD COLUMN dedup_key TEXT;

CREATE UNIQUE INDEX uq_mail_queue_dedup_key
    ON mail_queue (dedup_key)
    WHERE dedup_key IS NOT NULL;
