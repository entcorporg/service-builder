# Capture des changements (CDC)

| | |
| --- | --- |
| Branche | `feature/capture-de-changements` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Debezium](https://github.com/debezium/debezium) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Publier sur Kafka les insertions, mises à jour et suppressions d'une table en lisant le journal de PostgreSQL, sans étape *Publier*.

## Pourquoi c'est pertinent pour Service Builder

Complète l'outbox : utile pour les tables écrites hors des flux, ou pour alimenter projections et recherche.

## Portage vers Rust

Le format d'événement (avant/après/opération/source) vient de Debezium ; la lecture de la réplication logique existe en Rust dans `supabase/etl` (Apache-2.0).

## Intégration envisagée

Option « publier les changements » sur une table, topic `<service>.<table>.changes`, slot de réplication géré par le service.

## Autres projets regardés

- supabase/etl — Apache-2.0, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
