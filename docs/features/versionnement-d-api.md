# Versionnement d'API

| | |
| --- | --- |
| Branche | `feature/versionnement-d-api` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [Cadwyn](https://github.com/zmievsa/cadwyn) |
| Licence | MIT |
| Langage d'origine | Python |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Faire vivre plusieurs versions d'une route à partir d'un seul flux : chaque version décrit ses différences, les requêtes et réponses sont converties ; en-têtes `Deprecation` et `Sunset`.

## Pourquoi c'est pertinent pour Service Builder

Aucune trace dans le dépôt. Le générateur peut produire les conversions que les équipes maintiennent à la main.

## Portage vers Rust

Réécriture modérée : le modèle « changements de version » de Cadwyn (inspiré de Stripe) se transpose aux schémas du projet.

## Intégration envisagée

Versions datées dans le projet, un document OpenAPI par version.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
