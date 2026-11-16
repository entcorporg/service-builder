# Consommation parallèle et par lots

| | |
| --- | --- |
| Branche | `feature/consommation-parallele` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Confluent Parallel Consumer](https://github.com/confluentinc/parallel-consumer) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Traiter en parallèle les messages d'une même partition tout en gardant l'ordre par clé ; ou recevoir les messages par lots.

## Pourquoi c'est pertinent pour Service Builder

Le débit d'un consumer est aujourd'hui borné par le nombre de partitions.

## Portage vers Rust

Réécriture délicate : la validation des offsets doit rester correcte quand les messages se terminent dans le désordre.

## Intégration envisagée

Réglages « parallélisme » et « taille de lot » d'un consumer ; un handler par lot reçoit une liste.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
