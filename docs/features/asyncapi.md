# Document AsyncAPI des topics

| | |
| --- | --- |
| Branche | `feature/asyncapi` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [AsyncAPI (spécification)](https://github.com/asyncapi/spec) |
| Licence | Apache-2.0 |
| Langage d'origine | Spécification (outillage en TypeScript) |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Générer la description AsyncAPI des topics, messages, producteurs et consommateurs, comme OpenAPI 3.1 l'est pour HTTP.

## Pourquoi c'est pertinent pour Service Builder

Le pendant événementiel d'OpenAPI ; tout le contenu est déjà dans le modèle.

## Portage vers Rust

Très petite réécriture : un générateur de document de plus, sur le modèle de celui d'OpenAPI.

## Intégration envisagée

Fichier `asyncapi.yaml` dans l'export et dans l'onglet « Code généré ».

## Autres projets regardés

- EventCatalog — écarté comme source : Business Source License.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
