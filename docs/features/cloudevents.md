# Enveloppe CloudEvents

| | |
| --- | --- |
| Branche | `feature/cloudevents` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [CloudEvents SDK pour Rust](https://github.com/cloudevents/sdk-rust) |
| Licence | Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Publier et lire les événements au format CloudEvents (identifiant, source, type, date, schéma), en mode binaire par en-têtes Kafka.

## Pourquoi c'est pertinent pour Service Builder

Aucune trace dans le dépôt. C'est le format d'échange attendu par les outils et plateformes tiers (Knative, EventBridge, Azure Event Grid).

## Portage vers Rust

Dépendance directe, avec une liaison `rdkafka` fournie.

## Intégration envisagée

Option par topic ; les attributs se déduisent du modèle (type = nom du message, source = service).

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
