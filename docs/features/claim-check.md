# Charges utiles volumineuses (claim check)

| | |
| --- | --- |
| Branche | `feature/claim-check` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Apache Camel (EIP Claim Check)](https://github.com/apache/camel) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Déposer une charge utile trop lourde dans le stockage objet et ne publier que sa référence ; le consumer la récupère de façon transparente.

## Pourquoi c'est pertinent pour Service Builder

Kafka limite la taille des messages ; les blocs objet (S3) existent déjà, il manque l'assemblage automatique.

## Portage vers Rust

Petite réécriture : seuil de taille, dépôt, référence dans un en-tête, nettoyage à l'expiration.

## Intégration envisagée

Option « au-delà de N Ko » sur un topic, traitée dans le producer et le consumer du runtime.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
