# Montée de version des événements

| | |
| --- | --- |
| Branche | `feature/versions-d-evenements` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Axon Framework (upcasters)](https://github.com/AxonFramework/AxonFramework) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Convertir à la lecture un ancien événement vers la forme courante (champ renommé, découpé, valeur par défaut), sans réécrire l'historique.

## Pourquoi c'est pertinent pour Service Builder

Indispensable dès qu'on garde les événements longtemps (event sourcing, rejeu). Le journal des renommages donne déjà une partie des conversions.

## Portage vers Rust

Réécriture modérée : chaîne de convertisseurs par version, générée depuis l'historique des changements du schéma.

## Intégration envisagée

Numéro de version par message, convertisseurs générés, étape manuelle seulement pour les cas non déductibles.

## Autres projets regardés

- Marten — MIT, C# : même mécanisme côté PostgreSQL.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
