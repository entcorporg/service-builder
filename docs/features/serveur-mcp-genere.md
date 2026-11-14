# Serveur MCP du backend généré

| | |
| --- | --- |
| Branche | `feature/serveur-mcp-genere` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [SDK Rust du Model Context Protocol](https://github.com/modelcontextprotocol/rust-sdk) |
| Licence | En transition de MIT vers Apache-2.0 (d'après son fichier de licence) |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Exposer les routes et RPC choisis du backend comme outils MCP, pour que des agents IA l'utilisent avec les mêmes permissions que l'API.

## Pourquoi c'est pertinent pour Service Builder

L'application a déjà un serveur MCP pour éditer le projet ; ici c'est le backend exporté qui en reçoit un.

## Portage vers Rust

Dépendance directe ; les schémas d'entrée des outils se déduisent des schémas du projet.

## Intégration envisagée

Case « exposer en MCP » par route, service MCP généré avec authentification par jeton.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
