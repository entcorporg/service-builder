# Connexion OAuth 2 / OpenID Connect

| | |
| --- | --- |
| Branche | `feature/connexion-oidc` |
| Famille | Authentification |
| État | Identifiée, non implémentée |
| Référence open source | [Ory Kratos](https://github.com/ory/kratos) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Se connecter avec Google, GitHub, Microsoft ou le SSO d'une entreprise : flux code + PKCE, liaison au compte, configuration par tenant.

## Pourquoi c'est pertinent pour Service Builder

Le manque le plus visible de la palette d'authentification (mots de passe, TOTP, passkeys existent).

## Portage vers Rust

Le déroulé (état, nonce, liaison de compte, cas d'erreur) vient de Kratos ; le protocole est couvert par la crate `openidconnect` (MIT).

## Intégration envisagée

Étapes `OidcStart { provider }` et `OidcFinish`, table `identities`, fournisseurs déclarés dans le projet.

## Autres projets regardés

- Supabase Auth — MIT, Go : plus simple à lire.
- ramosbugs/openidconnect-rs — MIT, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
