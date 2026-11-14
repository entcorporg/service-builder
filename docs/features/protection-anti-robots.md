# Protection contre les robots

| | |
| --- | --- |
| Branche | `feature/protection-anti-robots` |
| Famille | Authentification |
| État | Identifiée, non implémentée |
| Référence open source | [ALTCHA](https://github.com/altcha-org/altcha) |
| Licence | MIT |
| Langage d'origine | TypeScript (bibliothèque serveur Rust : `altcha-lib-rs`, MIT) |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Un défi par preuve de travail, sans service tiers ni suivi, vérifié par un bloc avant l'inscription ou la connexion.

## Pourquoi c'est pertinent pour Service Builder

Le rate limit ne suffit pas contre l'inscription automatisée ; aucune trace dans le dépôt.

## Portage vers Rust

La bibliothèque serveur existe en Rust : création et vérification du défi.

## Intégration envisagée

Route de défi générée et étape `VerifyChallenge { payload }`.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
