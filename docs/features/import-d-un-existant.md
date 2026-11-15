# Importer un existant

| | |
| --- | --- |
| Branche | `feature/import-d-un-existant` |
| Famille | Application |
| État | Identifiée, non implémentée |
| Référence open source | [SeaSchema](https://github.com/SeaQL/sea-schema) |
| Licence | MIT ou Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Créer ou compléter un projet depuis une base PostgreSQL existante, un document OpenAPI, des fichiers `.proto` ou un document AsyncAPI.

## Pourquoi c'est pertinent pour Service Builder

Supprime la ressaisie pour qui migre un backend existant : c'est la porte d'entrée de la plupart des utilisateurs réels.

## Portage vers Rust

Découverte du schéma PostgreSQL en Rust ; `protox` lit déjà les `.proto` ; l'import OpenAPI est à écrire.

## Intégration envisagée

Assistant « Importer » : tables → tables du projet, routes → routes sans flux, messages → schémas.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
