# Routage, découpage et regroupement de messages

| | |
| --- | --- |
| Branche | `feature/routage-de-messages` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Apache Camel](https://github.com/apache/camel) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Les motifs d'intégration classiques comme types de consumer : routeur par contenu, filtre, découpeur (un message → plusieurs), agrégateur (plusieurs → un, par clé et délai).

## Pourquoi c'est pertinent pour Service Builder

Ces motifs se dessinent aujourd'hui à la main avec *Condition*, *Pour chaque* et *Publier* ; l'agrégateur, lui, demande du *Code Rust*.

## Portage vers Rust

Réécriture modérée : le routeur et le découpeur sont simples ; l'agrégateur demande un état persistant et une échéance.

## Intégration envisagée

Modèles de consumer prêts à l'emploi ; état de l'agrégateur dans PostgreSQL.

## Autres projets regardés

- Watermill — MIT, Go : implémentations courtes des mêmes motifs.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
