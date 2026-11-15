# Agrégats et auto-jointures dans le bloc SQL

| | |
| --- | --- |
| Branche | `feature/agregats-sql` |
| Famille | Bloc SQL |
| État | Identifiée, non implémentée |
| Référence open source | [PostgREST (fonctions d'agrégat)](https://github.com/PostgREST/postgrest) |
| Licence | MIT |
| Langage d'origine | Haskell |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

`GROUP BY`, `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `HAVING`, et la jointure d'une table à elle-même, dans le bloc SQL typé.

## Pourquoi c'est pertinent pour Service Builder

Limite écrite noir sur blanc dans `docs/ARCHITECTURE.md` : « un agrégat ou une jointure d'une table à elle-même » s'écrit aujourd'hui en *Code Rust*. Tout tableau de bord, total ou statistique y passe.

## Portage vers Rust

Rien à porter du Haskell : c'est une extension du générateur SQL et du typage (le type d'un `SUM` dépend de la colonne, une colonne non groupée est refusée à l'édition).

## Intégration envisagée

Section « regrouper par » et colonnes calculées dans l'éditeur du bloc SQL ; alias de table pour l'auto-jointure.

## Autres projets regardés

- Hasura graphql-engine — Apache-2.0 : agrégats exposés par relation.
- SeaQuery — MIT / Apache-2.0, Rust : constructeur de requêtes, utile comme modèle de l'arbre SQL.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
