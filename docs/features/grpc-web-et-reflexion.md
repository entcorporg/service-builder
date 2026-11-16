# gRPC-Web et réflexion gRPC

| | |
| --- | --- |
| Branche | `feature/grpc-web-et-reflexion` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [Connect](https://github.com/connectrpc/connect-go) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Rendre les RPC appelables depuis un navigateur et explorables avec `grpcurl` ou Postman.

## Pourquoi c'est pertinent pour Service Builder

Deux couches à activer dans le serveur généré : coût faible, confort immédiat.

## Portage vers Rust

Rien à porter : `tonic-web` et `tonic-reflection` (MIT) font le travail. Connect sert de référence pour le protocole et l'ergonomie.

## Intégration envisagée

Deux cases à cocher par service ; le jeu de descripteurs est déjà produit par `protox`.

## Autres projets regardés

- hyperium/tonic — MIT, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
