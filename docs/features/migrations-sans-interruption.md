# Migrations sans interruption de service

| | |
| --- | --- |
| Branche | `feature/migrations-sans-interruption` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [pgroll](https://github.com/xataio/pgroll) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Signaler à l'édition un changement de schéma qui bloquerait la base ou casserait l'ancienne version du service, et le découper en étapes (ajouter, remplir, basculer, retirer).

## Pourquoi c'est pertinent pour Service Builder

Le journal des renommages et conversions existe ; il ne dit pas encore si le changement est sûr pendant un déploiement progressif.

## Portage vers Rust

Le découpage en étapes vient de pgroll ; la détection des opérations dangereuses existe en Rust dans Squawk (MIT / Apache-2.0).

## Intégration envisagée

Diagnostics « migration bloquante » ; migrations générées en plusieurs temps.

## Autres projets regardés

- Squawk — MIT / Apache-2.0, Rust.
- strong_migrations — MIT, Ruby : le meilleur catalogue de cas dangereux.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
