# Compteurs d'usage et quotas

| | |
| --- | --- |
| Branche | `feature/quotas-et-usage` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [OpenMeter](https://github.com/openmeterio/openmeter) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Compter une consommation par tenant et par période (appels, sièges, stockage) et refuser au-delà d'un plafond lié à l'offre.

## Pourquoi c'est pertinent pour Service Builder

Le rate limit existe mais il est éphémère ; la facturation à l'usage demande un compteur persistant et interrogeable.

## Portage vers Rust

Réécriture modérée : événements d'usage idempotents, agrégation par fenêtre, droits (« entitlements »). OpenMeter s'appuie sur ClickHouse, à remplacer par PostgreSQL.

## Intégration envisagée

Table `usage_events` + vue agrégée, étapes `Meter { key, amount }` et `CheckQuota { key, limit }`.

## Autres projets regardés

- Lago — écarté comme source : AGPL-3.0 (Ruby).

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
