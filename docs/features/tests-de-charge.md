# Tests de charge générés

| | |
| --- | --- |
| Branche | `feature/tests-de-charge` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Goose](https://github.com/tag1consulting/goose) |
| Licence | Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Un scénario de charge par route, avec des données valides tirées des schémas, lançable contre l'export.

## Pourquoi c'est pertinent pour Service Builder

Le débit sous charge est cité comme non vérifié dans la documentation du service mail ; même besoin pour chaque service.

## Portage vers Rust

Dépendance d'un binaire de test généré dans le workspace.

## Intégration envisagée

Dossier `loadtests/` dans l'export, scénarios déduits des routes et de leurs exemples. Variante à proposer en parallèle : générer des scripts k6 (un fichier JavaScript par scénario), pour les équipes qui ont déjà k6.

## Autres projets regardés

- k6 — AGPL-3.0, Go : on ne reprend pas son code, mais les scripts générés sont les nôtres et l'utilisateur lance k6 lui-même ; la licence ne s'applique donc pas à l'export.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
