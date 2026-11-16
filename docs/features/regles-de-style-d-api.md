# Règles de style d'API

| | |
| --- | --- |
| Branche | `feature/regles-de-style-d-api` |
| Famille | Application |
| État | Identifiée, non implémentée |
| Référence open source | [vacuum](https://github.com/daveshanley/vacuum) |
| Licence | MIT |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Des règles configurables vérifiées à l'édition : nommage des routes et champs, pagination obligatoire sur les listes, format des erreurs, description requise.

## Pourquoi c'est pertinent pour Service Builder

La validation actuelle vérifie la justesse, pas la cohérence d'une API entre dix services et plusieurs auteurs.

## Portage vers Rust

Petite réécriture : reprendre le catalogue de règles (compatible Spectral) et l'appliquer au modèle plutôt qu'au document OpenAPI.

## Intégration envisagée

Jeu de règles dans le projet, diagnostics `W_STYLE_*` désactivables un par un.

## Autres projets regardés

- Spectral — Apache-2.0, TypeScript : le format de règles d'origine.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
