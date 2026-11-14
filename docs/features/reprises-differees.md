# Reprises différées et messages retardés

| | |
| --- | --- |
| Branche | `feature/reprises-differees` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Spring Kafka (retry topics non bloquants)](https://github.com/spring-projects/spring-kafka) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Rejouer un message en échec après 1 min, 10 min, 1 h sur des topics de reprise, sans bloquer la partition ; publier un message à livrer plus tard.

## Pourquoi c'est pertinent pour Service Builder

Aujourd'hui un message échoue 3 fois puis part en DLQ : les pannes passagères d'une dépendance remplissent la DLQ.

## Portage vers Rust

Réécriture modérée dans `runtime/` : topics `<topic>.retry.<délai>`, en-têtes de tentative, pause du consumer jusqu'à l'échéance.

## Intégration envisagée

Politique de reprise par consumer dans l'inspecteur, option `deliverAt` sur *Publier*.

## Autres projets regardés

- Watermill — MIT, Go : middlewares de reprise plus simples à lire.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
