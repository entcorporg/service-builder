-- Les mails, éditables sans redéploiement. Clé : `event_type` = topic de l'événement (comme `mail_queue.event_type`).
-- `subject`, `html_body` et `text_body` sont des templates Tera rendus avec les champs de l'événement, tels que
-- déclarés dans son schéma. Les mails du projet sont semés au démarrage (`src/seed.rs`).

CREATE TABLE mail_templates (
    event_type  TEXT        PRIMARY KEY,
    subject     TEXT        NOT NULL,
    html_body   TEXT        NOT NULL,
    text_body   TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
