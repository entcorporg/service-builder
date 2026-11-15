# Clients dans d'autres langages

| | |
| --- | --- |
| Branche | `feature/clients-multi-langages` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [OpenAPI Generator](https://github.com/OpenAPITools/openapi-generator) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Exporter des clients Python, Go, Kotlin, Swift ou C# des routes HTTP et des RPC.

## Pourquoi c'est pertinent pour Service Builder

Prolonge la fiche *Client TypeScript typé* pour les clients mobiles et les intégrateurs.

## Portage vers Rust

Rien à porter au départ : appeler l'outil sur l'OpenAPI exporté (et `protoc` pour gRPC). Une réécriture ne se justifie que langage par langage.

## Intégration envisagée

Choix des langages à l'export, dossier `clients/<langage>/`.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
