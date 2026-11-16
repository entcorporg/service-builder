# Identifiants : UUIDv7, identifiants courts, numéros sans trou

| | |
| --- | --- |
| Branche | `feature/identifiants` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [Sqids](https://github.com/sqids/sqids-rust) |
| Licence | MIT |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Choisir le type d'identifiant d'une table : UUIDv7 (triable), identifiant court opaque pour les URL, ou numéro séquentiel sans trou (factures).

## Pourquoi c'est pertinent pour Service Builder

Aucune trace dans le dépôt ; le numéro sans trou est une obligation comptable souvent mal faite.

## Portage vers Rust

Dépendances directes (`sqids`, `uuid` avec v7) ; le numéro sans trou est une table de compteurs verrouillée dans la transaction.

## Intégration envisagée

Type d'identifiant dans l'éditeur de table ; étape `NextNumber { sequence, scope }`.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
