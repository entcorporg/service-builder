# Tests de contrat entre services

| | |
| --- | --- |
| Branche | `feature/tests-de-contrat` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Pact (implémentation de référence)](https://github.com/pact-foundation/pact-reference) |
| Licence | MIT |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Vérifier qu'un service producteur respecte ce que ses consommateurs attendent, pour HTTP comme pour les messages.

## Pourquoi c'est pertinent pour Service Builder

Utile quand une partie des services n'est pas générée par service-builder.

## Portage vers Rust

Le cœur de Pact est en Rust (`pact_consumer`, `pact_verifier`).

## Intégration envisagée

Fichiers de contrat produits à l'export, vérification dans les tests générés.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
