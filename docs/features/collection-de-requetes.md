# Collection de requêtes exportée

| | |
| --- | --- |
| Branche | `feature/collection-de-requetes` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [Bruno](https://github.com/usebruno/bruno) |
| Licence | MIT |
| Langage d'origine | JavaScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Exporter une collection prête à l'emploi (requêtes, environnements, exemples) pour essayer l'API à la main.

## Pourquoi c'est pertinent pour Service Builder

Petit confort ; tout est déjà dans le modèle et l'OpenAPI.

## Portage vers Rust

Très petite réécriture : émettre des fichiers texte au format de Bruno, ou des fichiers `.http`.

## Intégration envisagée

Dossier `requests/` dans l'export.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
