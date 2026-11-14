# Règles métier et tables de décision

| | |
| --- | --- |
| Branche | `feature/regles-metier` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [GoRules Zen](https://github.com/gorules/zen) |
| Licence | MIT |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Une table de décision éditable (conditions en colonnes, résultats en lignes) évaluée par un bloc, à la place de cascades de *Condition*.

## Pourquoi c'est pertinent pour Service Builder

Tarifs, éligibilité, routage : la logique qui pousse aujourd'hui vers le *Code Rust*.

## Portage vers Rust

Moteur Rust embarquable (format JDM). Choix à faire : l'embarquer tel quel, ou compiler la table en `match` Rust pour rester sans runtime.

## Intégration envisagée

Éditeur de table dans l'inspecteur, étape `Decide { table, input, bind }`, typage des colonnes par le système existant.

## Autres projets regardés

- DMN (norme OMG) — pour le vocabulaire des politiques de résolution.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
