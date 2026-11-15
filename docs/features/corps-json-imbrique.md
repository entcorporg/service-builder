# Corps JSON imbriqués pour les appels HTTP

| | |
| --- | --- |
| Branche | `feature/corps-json-imbrique` |
| Famille | Option d'étape |
| État | Identifiée, non implémentée |
| Référence open source | [JSONata](https://github.com/jsonata-js/jsonata) |
| Licence | MIT |
| Langage d'origine | JavaScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Construire et lire des objets et listes imbriqués dans le corps d'un `httpCall` (et d'une réponse), champ à champ et typés.

## Pourquoi c'est pertinent pour Service Builder

Limite documentée : « pas de JSON imbriqué » pour `httpCall`, d'où un passage par le *Code Rust* pour la plupart des API tierces réelles.

## Portage vers Rust

Ne pas porter le langage entier : étendre les mappings typés existants aux objets et listes imbriqués, en s'inspirant de JSONata pour les chemins et les projections de listes.

## Intégration envisagée

Éditeur de mapping en arbre dans l'inspecteur de `httpCall` ; schéma de la réponse déclaré ou importé d'un exemple.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
