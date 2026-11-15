# Connexions aux comptes tiers des utilisateurs

| | |
| --- | --- |
| Branche | `feature/connexions-a-des-api-tierces` |
| Famille | Authentification |
| État | Identifiée, non implémentée |
| Référence open source | [oauth2 (crate)](https://github.com/ramosbugs/oauth2-rs) |
| Licence | MIT ou Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Laisser un utilisateur relier son compte YouTube, Google, Stripe ou autre, conserver les jetons chiffrés, les renouveler, et appeler l'API tierce en son nom depuis un flux.

## Pourquoi c'est pertinent pour Service Builder

Différent de la connexion OIDC : ici on agit pour l'utilisateur chez un tiers. `httpCall` sait appeler une API mais pas obtenir ni renouveler un jeton par utilisateur.

## Portage vers Rust

Le protocole est couvert ; reste le coffre de jetons (chiffrement déjà disponible), le renouvellement et la révocation.

## Intégration envisagée

Fournisseurs déclarés dans le projet, table `connections`, mode d'authentification « connexion de l'utilisateur » sur `httpCall`.

## Autres projets regardés

- Nango — écarté comme source : Elastic License 2.0.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
