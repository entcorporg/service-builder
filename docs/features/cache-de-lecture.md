# Cache de lecture

| | |
| --- | --- |
| Branche | `feature/cache-de-lecture` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [Moka](https://github.com/moka-rs/moka) |
| Licence | MIT ou Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Mettre en cache le résultat d'une requête ou d'un appel, avec durée de vie et invalidation par événement.

## Pourquoi c'est pertinent pour Service Builder

Seul l'en-tête `Cache-Control` existe ; rien côté serveur.

## Portage vers Rust

Dépendance directe pour le cache en mémoire ; variante partagée sur Redis à écrire.

## Intégration envisagée

Option « mettre en cache » sur *SQL*, *httpCall* et *CallRpc* ; étape `Invalidate { key }` ; invalidation déclarée sur un topic.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
