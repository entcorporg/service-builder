# Sauvegardes et restauration

| | |
| --- | --- |
| Branche | `feature/sauvegardes-et-restauration` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [pgBackRest](https://github.com/pgbackrest/pgbackrest) |
| Licence | MIT |
| Langage d'origine | C |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Configurer la sauvegarde continue de chaque base, la restauration à un instant donné, et un test de restauration planifié.

## Pourquoi c'est pertinent pour Service Builder

Aucune trace dans le dépôt ; une sauvegarde jamais restaurée n'en est pas une.

## Portage vers Rust

Rien à porter : générer la configuration et le conteneur, à relier aux fiches Kubernetes et OpenTofu.

## Intégration envisagée

Réglage « conserver N jours » par base ; job de test de restauration.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
