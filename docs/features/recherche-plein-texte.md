# Recherche plein texte

| | |
| --- | --- |
| Branche | `feature/recherche-plein-texte` |
| Famille | Bloc SQL |
| État | Identifiée, non implémentée |
| Référence open source | [PostgREST (opérateurs `fts`, `plfts`, `phfts`, `wfts`)](https://github.com/PostgREST/postgrest) |
| Licence | MIT |
| Langage d'origine | Haskell |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Un index de recherche sur des colonnes de texte et un opérateur « Recherche » avec classement, dans le bloc SQL.

## Pourquoi c'est pertinent pour Service Builder

Aucune trace dans le dépôt. Reste dans PostgreSQL (`tsvector`), sans moteur externe à exploiter.

## Portage vers Rust

Petite réécriture : il s'agit de générer du SQL (`to_tsvector`, `websearch_to_tsquery`, index GIN), rien à porter du Haskell.

## Intégration envisagée

Type d'index « recherche » sur la table, opérateur `Search` à côté de `Contains` et `StartsWith`, choix de la langue.

## Autres projets regardés

- Meilisearch — MIT (parties entreprise sous BSL), Rust : alternative si l'on veut tolérance aux fautes et facettes, au prix d'un service de plus.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
