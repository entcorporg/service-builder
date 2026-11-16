# WebSocket bidirectionnel

| | |
| --- | --- |
| Branche | `feature/websocket` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [Centrifugo](https://github.com/centrifugal/centrifugo) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Un canal où le navigateur envoie aussi des messages : abonnements dynamiques, présence, accusés de réception.

## Pourquoi c'est pertinent pour Service Builder

Le service temps réel est en SSE (serveur → client). Le WebSocket n'apporte que le sens inverse : à faire si un cas d'usage le demande.

## Portage vers Rust

Réécriture modérée : le protocole (abonnement, reprise après coupure, historique) se reprend ; axum fournit le WebSocket.

## Intégration envisagée

Second transport du service temps réel, mêmes règles d'audience que `PublishLive`.

## Autres projets regardés

- Supabase Realtime — Apache-2.0, Elixir.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
