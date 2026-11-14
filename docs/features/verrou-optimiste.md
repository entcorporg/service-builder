# Verrou optimiste et requêtes conditionnelles

| | |
| --- | --- |
| Branche | `feature/verrou-optimiste` |
| Famille | Option de table |
| État | Identifiée, non implémentée |
| Référence open source | [Ecto (`optimistic_lock`)](https://github.com/elixir-ecto/ecto) |
| Licence | Apache-2.0 |
| Langage d'origine | Elixir |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Une colonne `version` incrémentée à chaque écriture, exposée en `ETag` ; `If-Match` renvoie 412 si la ligne a changé entre-temps.

## Pourquoi c'est pertinent pour Service Builder

Évite les écrasements silencieux entre deux onglets ou deux services. SQL et en-têtes de réponse sont déjà générés.

## Portage vers Rust

Très petite réécriture : une clause `WHERE version = $n` et un contrôle du nombre de lignes touchées.

## Intégration envisagée

Case à cocher sur la table, génération de l'ETag, erreur 412/428 dans OpenAPI.

## Autres projets regardés

- RFC 9110 (requêtes conditionnelles) — la référence pour le comportement HTTP.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
