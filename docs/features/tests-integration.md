# Tests d'intégration générés

| | |
| --- | --- |
| Branche | `feature/tests-integration` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Testcontainers pour Rust](https://github.com/testcontainers/testcontainers-rs) |
| Licence | MIT ou Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Des tests qui lancent Kafka et PostgreSQL, démarrent les services générés et vérifient un scénario de bout en bout.

## Pourquoi c'est pertinent pour Service Builder

Point 1 de la feuille de route, et l'ajout le plus rentable : il prouve que ce qui est dessiné fonctionne.

## Portage vers Rust

Dépendance de test du workspace généré.

## Intégration envisagée

Scénarios décrits dans le projet (requête → événements et lignes attendus), dossier `tests/` dans l'export.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
