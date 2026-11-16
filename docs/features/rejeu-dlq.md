# Rejouer une DLQ ou un topic

| | |
| --- | --- |
| Branche | `feature/rejeu-dlq` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Kafbat UI](https://github.com/kafbat/kafka-ui) |
| Licence | Apache-2.0 |
| Langage d'origine | Java (interface en TypeScript) |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Lister les messages en DLQ avec leur erreur, les rejouer un par un ou en lot, et remettre un consumer à une position ou une date.

## Pourquoi c'est pertinent pour Service Builder

La DLQ existe mais rien ne permet d'en sortir sans outil externe ; l'onglet Pods est l'endroit naturel.

## Portage vers Rust

Petite réécriture au-dessus de `rdkafka` : lecture d'un topic, republication, réinitialisation des offsets.

## Intégration envisagée

Vue « DLQ » dans l'onglet Pods et route d'administration générée, protégée par permission.

## Autres projets regardés

- AKHQ — Apache-2.0, Java.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
