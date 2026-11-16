# Injection de pannes

| | |
| --- | --- |
| Branche | `feature/injection-de-pannes` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Toxiproxy](https://github.com/Shopify/toxiproxy) |
| Licence | MIT |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Dans l'onglet Pods : couper, ralentir ou dégrader le lien vers Kafka, PostgreSQL ou un autre service, et observer reprises, DLQ et outbox.

## Pourquoi c'est pertinent pour Service Builder

Les garanties annoncées (at-least-once, outbox, idempotence) ne se vérifient qu'en provoquant des pannes.

## Portage vers Rust

Rien à porter au départ : lancer Toxiproxy comme conteneur entre les services et le piloter par son API HTTP.

## Intégration envisagée

Commandes « couper » et « ralentir » sur une arête du graphe dans l'onglet Pods ; scénarios rejouables dans les tests d'intégration.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
