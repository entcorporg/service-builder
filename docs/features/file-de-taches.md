# File de tâches en arrière-plan

| | |
| --- | --- |
| Branche | `feature/file-de-taches` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [River](https://github.com/riverqueue/river) |
| Licence | MPL-2.0 (copyleft limité au fichier : s'en inspirer, ne pas copier de fichier) |
| Langage d'origine | Go |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Mettre en file un travail depuis un flux (dans la même transaction), avec priorité, reprises, unicité et exécution différée.

## Pourquoi c'est pertinent pour Service Builder

Les jobs actuels sont périodiques ; il manque « fais ceci plus tard, une seule fois, et réessaie ». À recouper avec ce que l'outbox fait déjà.

## Portage vers Rust

Le modèle (table de jobs, `FOR UPDATE SKIP LOCKED`, états, unicité) vient de River ; `apalis` (MIT) est une base Rust possible.

## Intégration envisagée

Table `jobs`, étape `Enqueue { job, args, runAt }`, workers déclarés comme les consumers.

## Autres projets regardés

- Oban — Apache-2.0, Elixir : l'original dont River s'inspire.
- graphile/worker — MIT, TypeScript.
- geofmureithi/apalis — MIT, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
