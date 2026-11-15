# Tests de robustesse et d'isolation générés

| | |
| --- | --- |
| Branche | `feature/tests-de-fuzzing-et-d-isolation` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Schemathesis](https://github.com/schemathesis/schemathesis) |
| Licence | MIT |
| Langage d'origine | Python |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Pour chaque route : entrées invalides et limites tirées du schéma, vérification que la réponse respecte le contrat, et tentatives d'accès sans jeton, sans permission ou depuis un autre tenant.

## Pourquoi c'est pertinent pour Service Builder

Le générateur connaît chaque route, sa permission et son tenant : il peut écrire les tests d'accès croisé que presque personne n'écrit à la main.

## Portage vers Rust

Deux voies : lancer Schemathesis sur l'OpenAPI exporté (dépendance Python), ou générer des tests Rust avec `proptest` ; les tests d'isolation sont propres à Service Builder et s'écrivent dans tous les cas.

## Intégration envisagée

Dossier `tests/` de l'export, à côté des tests d'intégration.

## Autres projets regardés

- RESTler — MIT, Python et F# : enchaîne les requêtes dépendantes.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
