# SSO SAML et provisionnement SCIM

| | |
| --- | --- |
| Branche | `feature/sso-saml-et-scim` |
| Famille | Authentification |
| État | Identifiée, non implémentée |
| Référence open source | [Ory Polis (ex-BoxyHQ SAML Jackson)](https://github.com/ory/polis) |
| Licence | Apache-2.0 |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Connexion par le fournisseur d'identité d'une entreprise en SAML, et création/suppression automatique des comptes par SCIM 2.0.

## Pourquoi c'est pertinent pour Service Builder

Condition d'achat fréquente des grands comptes ; complète la fiche *Connexion OAuth 2 / OpenID Connect*.

## Portage vers Rust

SAML : la crate `samael` (MIT) couvre le protocole. SCIM : routes à générer, c'est un CRUD normalisé sur les comptes et groupes.

## Intégration envisagée

Connexion SSO par tenant, routes `/scim/v2/Users` et `/Groups` générées sur les tables de comptes.

## Autres projets regardés

- Keycloak — Apache-2.0, Java : référence de comportement.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
