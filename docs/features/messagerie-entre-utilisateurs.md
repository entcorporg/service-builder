# Messagerie entre utilisateurs

| | |
| --- | --- |
| Branche | `feature/messagerie-entre-utilisateurs` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [Chatwoot](https://github.com/chatwoot/chatwoot) |
| Licence | MIT (hors dossier entreprise) |
| Langage d'origine | Ruby |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Des conversations entre deux parties ou plus : fils, messages, pièces jointes, lu/non lu, notification si le destinataire est absent, signalement.

## Pourquoi c'est pertinent pour Service Builder

Brique de toute place de marché ou outil de mise en relation. Le service temps réel et les notifications existent ; pas le modèle de conversation.

## Portage vers Rust

Réécriture modérée : surtout un modèle de données et des flux assemblés à partir de blocs existants (SQL, `PublishLive`, notification).

## Intégration envisagée

Modèle de service « messagerie » prêt à l'emploi plutôt que de nouvelles étapes.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
