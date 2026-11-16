# Partitionnement et rétention des tables

| | |
| --- | --- |
| Branche | `feature/partitionnement-et-retention` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [pg_partman](https://github.com/pgpartman/pg_partman) |
| Licence | Licence PostgreSQL (permissive) |
| Langage d'origine | PL/pgSQL et C (extension PostgreSQL) |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Partitionner une table par date et supprimer ou archiver automatiquement les partitions anciennes.

## Pourquoi c'est pertinent pour Service Builder

Tables d'événements, d'audit et d'usage grossissent sans fin ; la purge ligne à ligne coûte cher.

## Portage vers Rust

Petite réécriture : générer le SQL de partitionnement natif et un job d'entretien, sans dépendre de l'extension.

## Intégration envisagée

Réglages « partitionner par » et « conserver N jours » sur une table ; job d'entretien généré.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
