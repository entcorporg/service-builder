# Agrégations par fenêtre et jointures de flux

| | |
| --- | --- |
| Branche | `feature/agregations-par-fenetre` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Kafka Streams](https://github.com/apache/kafka) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Compter, sommer ou joindre des événements sur une fenêtre de temps (5 min glissantes, par jour) et publier le résultat.

## Pourquoi c'est pertinent pour Service Builder

Permet détection de fraude, tableaux de bord et alertes sans écrire de *Code Rust*.

## Portage vers Rust

Grosse réécriture si l'on veut l'exactitude (état local, heure d'événement, retardataires). Arroyo (Rust) montre comment le faire.

## Intégration envisagée

Type de consumer « agrégation » : clé, fenêtre, fonction, topic de sortie ; état dans PostgreSQL pour commencer.

## Autres projets regardés

- Arroyo — Apache-2.0 / MIT, Rust : la meilleure lecture pour une implémentation Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
