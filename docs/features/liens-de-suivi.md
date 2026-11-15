# Liens de suivi et attribution

| | |
| --- | --- |
| Branche | `feature/liens-de-suivi` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [Kutt](https://github.com/thedevs-network/kutt) |
| Licence | MIT |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Créer des liens courts et des codes promotionnels rattachés à une campagne, compter clics et conversions, et attribuer un résultat à sa source.

## Pourquoi c'est pertinent pour Service Builder

Nécessaire au sponsoring, à l'affiliation et au parrainage : c'est ce qui permet de payer à la performance.

## Portage vers Rust

Petite réécriture : redirection, comptage par événement, dédoublonnage. Les compteurs passent par la fiche *Compteurs d'usage et quotas*.

## Intégration envisagée

Route de redirection générée, topic `link.clicked`, étape `TrackConversion { code }`.

## Autres projets regardés

- Dub — non retenu : AGPL-3.0 à ma connaissance, non vérifié.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
