# Machine à états

| | |
| --- | --- |
| Branche | `feature/machine-a-etats` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [XState](https://github.com/statelyai/xstate) |
| Licence | MIT |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Déclarer les statuts d'une entité et les transitions permises ; un bloc « Transition » refuse tout changement non déclaré et peut publier un événement.

## Pourquoi c'est pertinent pour Service Builder

Les transitions se valident à l'édition, comme les mappings typés. Évite les `if status == …` dispersés dans les flux.

## Portage vers Rust

Petite réécriture : seul le sous-ensemble états/transitions/gardes est utile, il se génère en `match` Rust exhaustif.

## Intégration envisagée

Déclaration sur une colonne de table, étape `Transition { table, id, to }`, diagnostic si un état est inatteignable.

## Autres projets regardés

- Medusa (MIT, TypeScript) — exemples réels sur les commandes.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
