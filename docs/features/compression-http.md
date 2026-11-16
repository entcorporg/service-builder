# Compression des réponses HTTP

| | |
| --- | --- |
| Branche | `feature/compression-http` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [tower-http](https://github.com/tower-rs/tower-http) |
| Licence | MIT |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

gzip, brotli et zstd négociés par `Accept-Encoding`.

## Pourquoi c'est pertinent pour Service Builder

Aucune trace dans le dépôt ; quasi gratuit.

## Portage vers Rust

Une couche `CompressionLayer` à ajouter au routeur généré.

## Intégration envisagée

Réglage par service, exclusion des flux SSE.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
