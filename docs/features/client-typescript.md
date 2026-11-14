# Client TypeScript typé

| | |
| --- | --- |
| Branche | `feature/client-typescript` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [openapi-typescript](https://github.com/openapi-ts/openapi-typescript) |
| Licence | MIT |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Exporter un client TypeScript des routes HTTP (et du flux temps réel), typé de bout en bout.

## Pourquoi c'est pertinent pour Service Builder

Un frontend consomme l'API sans friction ni dérive ; l'essentiel du bénéfice attendu de GraphQL pour une fraction du coût.

## Portage vers Rust

Deux voies : appeler l'outil sur l'OpenAPI généré (dépendance Node à l'export), ou réécrire un émetteur de types en Rust, petit puisque le modèle est connu.

## Intégration envisagée

Dossier `clients/typescript/` dans l'export.

## Autres projets regardés

- hey-api/openapi-ts — TypeScript (licence non vérifiée).

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
