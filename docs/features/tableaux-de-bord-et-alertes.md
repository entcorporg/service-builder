# Tableaux de bord et alertes générés

| | |
| --- | --- |
| Branche | `feature/tableaux-de-bord-et-alertes` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Sloth](https://github.com/slok/sloth) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Générer les objectifs de niveau de service, les règles d'alerte Prometheus et un tableau de bord par service à partir des routes et des consumers.

## Pourquoi c'est pertinent pour Service Builder

`/metrics` existe ; personne n'aime écrire les règles et tableaux qui vont avec, et le générateur connaît chaque route et chaque topic.

## Portage vers Rust

Petite réécriture : émettre du YAML de règles (alertes multi-fenêtres sur le budget d'erreur) et du JSON de tableau de bord.

## Intégration envisagée

Objectif par route ou consumer dans l'inspecteur ; dossier `observability/` dans l'export.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
