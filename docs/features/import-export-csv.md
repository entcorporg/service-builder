# Import et export CSV

| | |
| --- | --- |
| Branche | `feature/import-export-csv` |
| Famille | Données |
| État | Implémentée en partie : export `text/csv` d'une liste et import d'un corps CSV (options `responseFormat` et `bodyFormat` d'une route, voir `docs/ARCHITECTURE.md`, « CSV ») ; ni flux continu, ni import partiel, ni tâche de fond |
| Référence open source | [PostgREST (`Accept: text/csv`, insertion en masse)](https://github.com/PostgREST/postgrest) |
| Licence | MIT |
| Langage d'origine | Haskell |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Exporter une table ou une requête en CSV en flux continu ; importer un fichier avec validation ligne à ligne et rapport d'erreurs.

## Pourquoi c'est pertinent pour Service Builder

Demandé dans presque tous les back-offices ; les schémas donnent déjà la validation.

## Portage vers Rust

Le comportement HTTP vient de PostgREST ; la lecture et l'écriture passent par la crate `csv` (MIT / Unlicense).

## Intégration envisagée

Options « exporter en CSV » sur une route de liste et étape `ImportCsv { table, file }`, traitée en tâche de fond.

## Autres projets regardés

- Directus — écarté comme source : licence MSCL.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
