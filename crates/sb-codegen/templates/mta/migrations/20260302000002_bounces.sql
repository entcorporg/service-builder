-- Table des bounces (NDR/DSN reçus sur le serveur SMTP entrant)
-- bounce_type:
--   permanent  → 5xx (adresse inexistante, boîte pleine définitive…)
--   transient  → 4xx (temporaire, reçu après nos tentatives maximales)

CREATE TABLE bounces (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id  TEXT        NOT NULL,                   -- corrélé avec mail_queue.message_id
    bounce_type TEXT        NOT NULL CHECK (bounce_type IN ('permanent', 'transient')),
    bounce_code INT,                                    -- code SMTP (ex: 550, 421)
    raw_dsn     TEXT,                                   -- corps brut du DSN pour diagnostic
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_bounces_message_id ON bounces (message_id);
