# API GraphQL générée

| | |
| --- | --- |
| Branche | `feature/graphql` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [PostGraphile (dépôt `graphile/crystal`)](https://github.com/graphile/crystal) |
| Licence | MIT |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Types, requêtes, mutations et pagination générés depuis les schémas et les tables, avec chargement groupé contre le N+1.

## Pourquoi c'est pertinent pour Service Builder

Demandé par les frontends riches. Gros chantier : permissions, tenant et pagination doivent être portés dans une seconde surface.

## Portage vers Rust

PostGraphile donne les conventions (nommage, connexions Relay, filtres) ; l'exécution se génère sur `async-graphql` (MIT / Apache-2.0).

## Intégration envisagée

Option « exposer en GraphQL » par service, résolveurs qui appellent les mêmes flux que les routes HTTP.

## Autres projets regardés

- Hasura graphql-engine — Apache-2.0, moteur v3 en Rust : utile pour les permissions par ligne.
- async-graphql — MIT / Apache-2.0, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
