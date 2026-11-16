# Tâches planifiées par expression cron

| | |
| --- | --- |
| Branche | `feature/taches-cron` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Oban (plugin Cron)](https://github.com/oban-bg/oban) |
| Licence | Apache-2.0 |
| Langage d'origine | Elixir |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Planifier « chaque lundi à 8 h, heure de Paris » en plus de « toutes les N secondes », avec une seule exécution quel que soit le nombre d'instances.

## Pourquoi c'est pertinent pour Service Builder

`spawn_periodic` ne connaît que l'intervalle. Demande les fuseaux horaires (fiche dédiée).

## Portage vers Rust

L'analyse des expressions existe en Rust (crate `croner`) ; reste à reprendre la garantie d'exécution unique, par verrou consultatif déjà utilisé dans le dépôt.

## Intégration envisagée

Champ « planification » d'un job : intervalle ou cron + fuseau ; validation de l'expression à l'édition.

## Autres projets regardés

- River (jobs périodiques) — MPL-2.0, Go.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
