# Copie anonymisée des données

| | |
| --- | --- |
| Branche | `feature/donnees-anonymisees` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [Greenmask](https://github.com/GreenmaskIO/greenmask) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Produire une copie de la base où les colonnes personnelles sont remplacées par des valeurs plausibles et cohérentes entre tables.

## Pourquoi c'est pertinent pour Service Builder

Permet de reproduire un bug avec des données réalistes sans exposer personne. S'appuie sur le marquage des colonnes de la fiche *Effacement et export RGPD*.

## Portage vers Rust

Rien à porter au départ : générer la configuration de Greenmask depuis le modèle. Une réécriture ne se justifie que pour l'intégrer à l'onglet Pods.

## Intégration envisagée

Fichier de configuration dans l'export ; règle de transformation par type de donnée personnelle.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
