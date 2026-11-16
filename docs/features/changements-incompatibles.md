# Détection des changements incompatibles

| | |
| --- | --- |
| Branche | `feature/changements-incompatibles` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Buf (`buf breaking`)](https://github.com/bufbuild/buf) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Comparer les `.proto`, les topics et les routes à la dernière version publiée et bloquer l'export sur une rupture de compatibilité.

## Pourquoi c'est pertinent pour Service Builder

Point 3 de la feuille de route ; le journal des renommages et suppressions fournit déjà la matière.

## Portage vers Rust

Réécriture modérée : reprendre le catalogue de règles (WIRE, WIRE_JSON, FILE) ; `protox` donne déjà les descripteurs à comparer.

## Intégration envisagée

Diagnostic « rupture » dans la validation, instantané des contrats à chaque export.

## Autres projets regardés

- oasdiff — Apache-2.0, Go : même chose pour OpenAPI.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
