# Connecteurs sources et destinations

| | |
| --- | --- |
| Branche | `feature/connecteurs` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Bento](https://github.com/warpstreamlabs/bento) |
| Licence | MIT |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Relier un topic à un système externe sans flux : S3, HTTP, base de données, fichier, autre broker ; en entrée comme en sortie.

## Pourquoi c'est pertinent pour Service Builder

Évite un service généré entier pour « copier ce topic vers S3 ». Recoupe en partie la capture des changements.

## Portage vers Rust

Grosse réécriture si l'on vise un catalogue ; plus raisonnable : quelques connecteurs natifs (S3, HTTP, PostgreSQL), ou un conteneur Bento ajouté au `docker-compose.yml` et configuré depuis le projet.

## Intégration envisagée

Nœud « connecteur » dans le graphe d'architecture, entre un topic et un système externe.

## Autres projets regardés

- Vector — MPL-2.0, Rust : s'en inspirer, ne pas copier de fichier.
- Redpanda Connect — licence non vérifiée (Bento en est le fork resté sous MIT).

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
