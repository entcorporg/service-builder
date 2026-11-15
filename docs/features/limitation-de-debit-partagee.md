# Limitation de débit partagée entre instances

| | |
| --- | --- |
| Branche | `feature/limitation-de-debit-partagee` |
| Famille | Option d'étape |
| État | Identifiée, non implémentée |
| Référence open source | [redis-cell](https://github.com/brandur/redis-cell) |
| Licence | MIT |
| Langage d'origine | Rust (module Redis) |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Compter les essais dans un magasin commun pour que la limite soit la même quel que soit le nombre de réplicas.

## Pourquoi c'est pertinent pour Service Builder

Limite documentée : le compte de `rateLimit` vit en mémoire de chaque instance, la limite réelle est donc multipliée par le nombre de réplicas.

## Portage vers Rust

L'algorithme GCRA se lit en Rust ; à réécrire en script Lua ou en commandes atomiques, car le service temps réel utilise déjà un magasin compatible Redis.

## Intégration envisagée

Option « partagé » sur l'étape `rateLimit`, repli en mémoire si le magasin est injoignable.

## Autres projets regardés

- Envoy ratelimit — Apache-2.0, Go.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
