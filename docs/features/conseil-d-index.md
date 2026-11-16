# Conseil d'index à l'édition

| | |
| --- | --- |
| Branche | `feature/conseil-d-index` |
| Famille | Bloc SQL |
| État | Identifiée, non implémentée |
| Référence open source | [index_advisor](https://github.com/supabase/index_advisor) |
| Licence | Licence PostgreSQL (permissive) |
| Langage d'origine | PL/pgSQL (au-dessus de HypoPG, en C) |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Signaler un bloc SQL qui filtre ou trie sur des colonnes sans index, et proposer l'index à créer.

## Pourquoi c'est pertinent pour Service Builder

L'application connaît toutes les requêtes et tous les index : elle peut prévenir avant la production, là où les autres outils constatent après.

## Portage vers Rust

Petite réécriture en analyse statique sur le modèle (colonnes des conditions, jointures et tris face aux index déclarés), sans base à interroger.

## Intégration envisagée

Avertissement `W_SQL_NO_INDEX` avec correction en un clic.

## Autres projets regardés

- HypoPG — licence PostgreSQL, C : index hypothétiques, pour une vérification sur base réelle dans l'onglet Pods.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
