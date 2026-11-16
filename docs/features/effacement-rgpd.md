# Effacement et export RGPD

| | |
| --- | --- |
| Branche | `feature/effacement-rgpd` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [Fides](https://github.com/ethyca/fides) |
| Licence | Apache-2.0 |
| Langage d'origine | Python |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Marquer les colonnes personnelles dans les schémas ; générer l'export des données d'une personne et son anonymisation à travers tous les services.

## Pourquoi c'est pertinent pour Service Builder

Le générateur connaît tout le modèle : il peut produire ce que les équipes écrivent d'ordinaire à la main, service par service.

## Portage vers Rust

Réécriture modérée : reprendre la taxonomie (catégories de données) et l'orchestration d'une demande ; la propagation entre services passe par un topic.

## Intégration envisagée

Attribut « donnée personnelle » sur un champ, topic `privacy.erasure`, consumer généré dans chaque service concerné.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
