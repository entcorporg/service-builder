# Requête-réponse sur Kafka

| | |
| --- | --- |
| Branche | `feature/requete-reponse-kafka` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Spring Kafka (`ReplyingKafkaTemplate`)](https://github.com/spring-projects/spring-kafka) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Publier une demande et attendre sa réponse sur un topic de retour, avec identifiant de corrélation et délai d'attente.

## Pourquoi c'est pertinent pour Service Builder

Aujourd'hui l'appel synchrone passe par gRPC ; ce bloc permet de rester sur le broker quand le service appelé n'expose pas de RPC. À recouper avec l'usage actuel des identifiants de corrélation dans le dépôt.

## Portage vers Rust

Réécriture modérée dans `runtime/` : en-têtes de corrélation et de topic de réponse, table d'attente en mémoire, expiration.

## Intégration envisagée

Étape `Request { topic, message, replyTopic, timeout, bind }` et option « répondre » sur un consumer.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
