# Webhooks sortants

| | |
| --- | --- |
| Branche | `feature/webhooks-sortants` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [Svix](https://github.com/svix/svix-webhooks) |
| Licence | MIT |
| Langage d'origine | Rust (`server/svix-server`) |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Envoyer des événements aux systèmes des clients : endpoints enregistrés par tenant, signature HMAC, reprises espacées, journal des livraisons, désactivation d'un endpoint qui échoue.

## Pourquoi c'est pertinent pour Service Builder

La vérification entrante existe (`runtime/webhook.rs`, schéma Stripe) et `httpCall` aussi, mais pas l'envoi fiable. L'outbox existante est la base naturelle.

## Portage vers Rust

Le serveur est en Rust (axum, sqlx) : le calendrier de reprises, la signature et le modèle endpoint/message/tentative se reprennent presque tels quels.

## Intégration envisagée

Tables `webhook_endpoints` et `webhook_attempts`, worker branché sur l'outbox, étape `SendWebhook { event, payload }`.

## Autres projets regardés

- Standard Webhooks — spécification ouverte de la signature, portée par Svix.
- Hook0 — écarté comme source : licence SSPL.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
