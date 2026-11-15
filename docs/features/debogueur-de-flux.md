# Débogueur de flux

| | |
| --- | --- |
| Branche | `feature/debogueur-de-flux` |
| Famille | Application |
| État | Identifiée, non implémentée |
| Référence open source | [Node-RED (panneau de débogage)](https://github.com/node-red/node-red) |
| Licence | Apache-2.0 |
| Langage d'origine | JavaScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Dans l'onglet Pods : suivre une requête ou un événement étape par étape, avec la valeur des variables, la durée et l'erreur de chaque étape.

## Pourquoi c'est pertinent pour Service Builder

Sans cela, comprendre pourquoi un flux dessiné échoue oblige à lire le Rust généré et les journaux.

## Portage vers Rust

Réécriture propre à Service Builder : instrumenter le code généré (en mode Pods seulement) pour émettre un événement par étape, et l'afficher sur les cartes d'étapes.

## Intégration envisagée

Surbrillance de l'étape en cours ou en échec dans l'éditeur ; s'appuie sur la fiche *Traçage OpenTelemetry*.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
