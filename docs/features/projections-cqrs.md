# Projections de lecture (CQRS)

| | |
| --- | --- |
| Branche | `feature/projections-cqrs` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Marten (projections)](https://github.com/JasperFx/marten) |
| Licence | MIT |
| Langage d'origine | C# |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Une table de lecture tenue à jour par un consumer généré, reconstructible depuis le début du topic ou du flux d'événements.

## Pourquoi c'est pertinent pour Service Builder

Dans la feuille de route. Évite d'écrire à la main le consumer, l'upsert et la reprise.

## Portage vers Rust

Réécriture modérée : reprendre le cycle de vie (en ligne, asynchrone, reconstruction, position du démon) ; l'upsert passe par le bloc SQL.

## Intégration envisagée

Déclaration « projection » : événements sources → colonnes ; commande de reconstruction ; suivi du retard.

## Autres projets regardés

- Materialize et RisingWave — vues incrémentales, mais ce sont des bases à part (RisingWave : Apache-2.0, Rust).

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
