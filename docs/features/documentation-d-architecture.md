# Documentation d'architecture générée

| | |
| --- | --- |
| Branche | `feature/documentation-d-architecture` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Mermaid](https://github.com/mermaid-js/mermaid) |
| Licence | MIT |
| Langage d'origine | JavaScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Diagrammes des services, topics et flux, catalogue des événements (qui publie, qui consomme), trajet de chaque donnée personnelle, et fiche d'exploitation par service.

## Pourquoi c'est pertinent pour Service Builder

Le graphe d'architecture existe dans l'application mais pas dans l'export ; cette documentation ne peut jamais dériver du code puisqu'elle en sort.

## Portage vers Rust

Rien à porter : émettre du texte Mermaid dans des fichiers Markdown, que GitHub et GitLab affichent directement.

## Intégration envisagée

Dossier `docs/` dans l'export : architecture, catalogue d'événements, un fichier par service.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
