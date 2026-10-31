-- Table de file d'attente pour tous les emails sortants
-- status:
--   pending  → en attente de traitement
--   sending  → en cours (verrou SKIP LOCKED)
--   sent     → livré avec succès
--   failed   → échecs max atteints
--   bounced  → NDR/DSN reçu du destinataire

CREATE TYPE mail_status AS ENUM ('pending', 'sending', 'sent', 'failed', 'bounced');

CREATE TABLE mail_queue (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type      TEXT        NOT NULL,
    recipient       TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    status          mail_status NOT NULL DEFAULT 'pending',
    attempts        INT         NOT NULL DEFAULT 0,
    max_attempts    INT         NOT NULL DEFAULT 5,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    message_id      TEXT,                                           -- Message-ID RFC 5322, positionné après construction MIME
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sent_at         TIMESTAMPTZ,
    error           TEXT                                            -- dernière erreur (SMTP ou DNS)
);

-- Index partiel : seuls les jobs pending/prochain essai sont scannés fréquemment
CREATE INDEX idx_mail_queue_pending
    ON mail_queue (next_attempt_at)
    WHERE status = 'pending';

-- Index sur message_id pour corrélation avec les bounces
CREATE INDEX idx_mail_queue_message_id
    ON mail_queue (message_id)
    WHERE message_id IS NOT NULL;
