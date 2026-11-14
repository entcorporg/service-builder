# Serveur simulé depuis OpenAPI

| | |
| --- | --- |
| Branche | `feature/serveur-simule` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [Prism](https://github.com/stoplightio/prism) |
| Licence | Apache-2.0 |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Un serveur qui répond avec des exemples conformes au contrat, pour que le frontend avance avant que les flux soient dessinés.

## Pourquoi c'est pertinent pour Service Builder

Utile dès qu'une route est déclarée mais pas encore câblée.

## Portage vers Rust

Petite réécriture si on l'intègre à l'onglet Pods (générer des valeurs depuis les schémas) ; sinon lancer Prism tel quel sur l'OpenAPI exporté.

## Intégration envisagée

Mode « simulé » d'un service dans l'onglet Pods.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
