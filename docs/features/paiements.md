# Paiements

| | |
| --- | --- |
| Branche | `feature/paiements` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [Hyperswitch](https://github.com/juspay/hyperswitch) |
| Licence | Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Créer un paiement, suivre son état et recevoir les notifications du prestataire, derrière une interface commune à plusieurs prestataires.

## Pourquoi c'est pertinent pour Service Builder

L'exemple livré a un service Payments, mais l'intégration d'un prestataire se fait à la main avec `httpCall` et un webhook signé.

## Portage vers Rust

Les connecteurs par prestataire et la machine à états d'un paiement se lisent en Rust. Deux voies : s'en inspirer pour un ou deux prestataires, ou appeler Hyperswitch comme service.

## Intégration envisagée

Modèle de service « paiements » : étapes `PaymentCreate`, `PaymentCapture`, `Refund`, webhook rattaché.

## Autres projets regardés

- Kill Bill — Apache-2.0, Java : abonnements et facturation, hors périmètre pour l'instant.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
