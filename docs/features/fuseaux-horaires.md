# Fuseaux horaires et types calendaires

| | |
| --- | --- |
| Branche | `feature/fuseaux-horaires` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [Jiff](https://github.com/BurntSushi/jiff) |
| Licence | MIT ou Unlicense |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Types date, heure locale, fuseau et durée calendaire ; conversions et arithmétique correctes aux changements d'heure.

## Pourquoi c'est pertinent pour Service Builder

Rien dans le dépôt ; indispensable aux tâches cron, aux rappels et aux rapports par jour.

## Portage vers Rust

Dépendance directe du code généré.

## Intégration envisagée

Types et fonctions d'expression (`toZone`, `startOfDay`, `addMonths`), fuseau par tenant.

## Autres projets regardés

- chrono-tz — MIT / Apache-2.0, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
