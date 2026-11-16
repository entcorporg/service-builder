# Point d'accès Peppol (AS4)

| | |
| --- | --- |
| Branche | `feature/point-d-acces-peppol` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [phase4](https://github.com/phax/phase4) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Envoyer et recevoir des documents sur le réseau Peppol : enveloppe SBDH, messagerie AS4 signée et chiffrée, recherche du destinataire (SML/SMP), accusés de réception.

## Pourquoi c'est pertinent pour Service Builder

C'est le canal d'interopérabilité entre plateformes de facturation. Protocole lourd (ebMS 3, WS-Security) et soumis à des tests de conformité.

## Portage vers Rust

Ne pas réécrire : pas d'implémentation Rust reconnue, et la conformité se prouve implémentation par implémentation. À intégrer comme service externe (voir la fiche *Services externes dans le graphe*), relié aux flux par des topics.

## Intégration envisagée

Deux topics générés (`peppol.outbound`, `peppol.inbound`) et un conteneur phase4 configuré depuis le projet.

## Autres projets regardés

- Oxalis — licence non vérifiée (fichier introuvable).

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
