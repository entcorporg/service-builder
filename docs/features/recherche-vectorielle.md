# Recherche vectorielle

| | |
| --- | --- |
| Branche | `feature/recherche-vectorielle` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [pgvector](https://github.com/pgvector/pgvector) |
| Licence | Licence PostgreSQL (permissive) |
| Langage d'origine | C (extension PostgreSQL) |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Un type de colonne vecteur, un index de proximité et un opérateur « plus proches voisins » dans le bloc SQL.

## Pourquoi c'est pertinent pour Service Builder

Recherche sémantique et recommandations sans quitter PostgreSQL.

## Portage vers Rust

Rien à porter : l'extension s'installe dans PostgreSQL, la crate `pgvector` (MIT / Apache-2.0) fournit le type pour sqlx.

## Intégration envisagée

Type `vector(n)`, index HNSW, opérateur `Nearest`, image PostgreSQL adaptée dans `docker-compose.yml`.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
