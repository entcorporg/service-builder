# Traductions des mails et des messages

| | |
| --- | --- |
| Branche | `feature/traductions` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [Fluent](https://github.com/projectfluent/fluent-rs) |
| Licence | MIT ou Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Traduire les modèles de mail, les messages d'erreur et les notifications selon la langue de la personne ou `Accept-Language`.

## Pourquoi c'est pertinent pour Service Builder

Presque rien dans le dépôt ; bloquant dès qu'un produit sort d'un seul pays.

## Portage vers Rust

Dépendance directe du code généré ; gère pluriels et genres mieux qu'un simple dictionnaire.

## Intégration envisagée

Fichiers `.ftl` par langue dans le projet, clé de message dans les modèles de mail et dans `Fail`, diagnostic des clés manquantes.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
