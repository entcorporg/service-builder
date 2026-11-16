# Event sourcing

| | |
| --- | --- |
| Branche | `feature/event-sourcing` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Marten](https://github.com/JasperFx/marten) |
| Licence | MIT |
| Langage d'origine | C# |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Stocker un agrégat comme une suite d'événements : flux par identifiant, contrôle de version à l'écriture, instantanés, relecture.

## Pourquoi c'est pertinent pour Service Builder

Brique centrale d'une architecture événementielle ; aujourd'hui les tables ne gardent que l'état courant.

## Portage vers Rust

Le schéma PostgreSQL de Marten (tables de flux et d'événements, numéros de séquence) est la référence ; côté Rust, `cqrs-es` (Apache-2.0) donne les traits agrégat/commande/événement.

## Intégration envisagée

Type de table « flux d'événements », étapes `AppendEvent` et `LoadAggregate`, publication par l'outbox.

## Autres projets regardés

- Axon Framework — Apache-2.0, Java.
- serverlesstechnology/cqrs — Apache-2.0, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
