# Autres brokers : NATS JetStream, RabbitMQ

| | |
| --- | --- |
| Branche | `feature/autres-brokers` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Watermill](https://github.com/ThreeDotsLabs/watermill) |
| Licence | MIT |
| Langage d'origine | Go |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Choisir le broker par projet, derrière la même abstraction producer/consumer que Kafka.

## Pourquoi c'est pertinent pour Service Builder

Kafka est lourd pour de petits déploiements ; NATS tient dans un binaire. Élargit nettement le public visé.

## Portage vers Rust

L'abstraction Publisher/Subscriber vient de Watermill ; les clients existent en Rust (`async-nats`, Apache-2.0 ; `lapin`, MIT).

## Intégration envisagée

Trait de transport dans `runtime/`, choix du broker dans les réglages du projet, `docker-compose.yml` adapté.

## Autres projets regardés

- nats-io/nats.rs — Apache-2.0, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
