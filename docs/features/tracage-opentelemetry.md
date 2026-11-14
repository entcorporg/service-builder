# Traçage OpenTelemetry

| | |
| --- | --- |
| Branche | `feature/tracage-opentelemetry` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust) |
| Licence | Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Une trace par requête, propagée à travers HTTP, gRPC, Kafka et l'outbox, exportée en OTLP.

## Pourquoi c'est pertinent pour Service Builder

Dans la feuille de route. Un générateur instrumente tout sans effort de l'utilisateur : chaque étape devient un segment nommé.

## Portage vers Rust

Dépendance directe ; le travail est dans la propagation de `traceparent` par les en-têtes Kafka et la table outbox.

## Intégration envisagée

Réglage par projet, collecteur dans `docker-compose.yml`.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
