# Chiffrement des événements par sujet

| | |
| --- | --- |
| Branche | `feature/chiffrement-par-sujet` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Kroxylicious (chiffrement d'enregistrements)](https://github.com/kroxylicious/kroxylicious) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Chiffrer les champs personnels d'un événement avec une clé propre à la personne ; supprimer la clé rend ses événements illisibles partout (crypto-shredding).

## Pourquoi c'est pertinent pour Service Builder

Seule façon d'honorer un effacement RGPD sur un journal immuable. Les blocs *Chiffrer* / *Déchiffrer* et la rotation de clés existent ; pas la clé par sujet.

## Portage vers Rust

Réécriture modérée : chiffrement par enveloppe (clé de données chiffrée par une clé maître), table des clés par sujet, cache.

## Intégration envisagée

Attribut « chiffré par sujet » sur un champ de message ; se combine avec la fiche *Effacement et export RGPD*.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
