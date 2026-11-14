# Politiques d'autorisation

| | |
| --- | --- |
| Branche | `feature/politiques-autorisation` |
| Famille | Authentification |
| État | Identifiée, non implémentée |
| Référence open source | [Cedar](https://github.com/cedar-policy/cedar) |
| Licence | Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Exprimer « qui peut faire quoi sur quelle ressource » par des politiques (rôle, propriétaire, attributs), vérifiées à l'édition et évaluées dans le flux.

## Pourquoi c'est pertinent pour Service Builder

Les permissions `module.view` / `module.manage` couvrent le cas courant mais pas « le propriétaire peut modifier sa ressource ».

## Portage vers Rust

Cedar est une bibliothèque Rust embarquable, avec un validateur de politiques par rapport à un schéma : il s'intègre sans service à part.

## Intégration envisagée

Fichier de politiques dans le projet, étape `Authorize { principal, action, resource }`, schéma Cedar dérivé des tables.

## Autres projets regardés

- OpenFGA — Apache-2.0, Go : modèle relationnel à la Zanzibar, demande un service dédié.
- Casbin — Apache-2.0, Go, portage `casbin-rs`.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
