# Requêtes géographiques

| | |
| --- | --- |
| Branche | `feature/requetes-geographiques` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [PostGIS](https://github.com/postgis/postgis) |
| Licence | GPL-2.0 (utilisée comme extension, sans copie de code : pas d'effet sur le code généré) |
| Langage d'origine | C / C++ (extension PostgreSQL) |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Types point et zone, et opérateurs « à moins de N mètres », « contenu dans ».

## Pourquoi c'est pertinent pour Service Builder

Livraison, magasins proches, zones de service.

## Portage vers Rust

Rien à porter : on génère du SQL ; `geo-types` et `geozero` couvrent les types côté Rust.

## Intégration envisagée

Types `geography(Point)` et opérateurs `Within`, `Near` dans le bloc SQL.

## Autres projets regardés

- `earthdistance` de PostgreSQL — suffisant pour de simples distances, sans extension lourde.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
