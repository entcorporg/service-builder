# Droits minimaux générés

| | |
| --- | --- |
| Branche | `feature/droits-minimaux` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Strimzi (ressources `KafkaTopic` et `KafkaUser`)](https://github.com/strimzi/strimzi-kafka-operator) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Donner à chaque service uniquement ce que le modèle dit qu'il utilise : ACL Kafka sur ses topics, rôle PostgreSQL sur ses tables, politique d'accès à ses objets.

## Pourquoi c'est pertinent pour Service Builder

Le modèle sait exactement qui publie, consomme, lit et écrit quoi ; aucune trace d'ACL ou de `GRANT` dans le dépôt.

## Portage vers Rust

Rien à porter : émettre des déclarations (ACL, `GRANT`, politiques) depuis le graphe du projet.

## Intégration envisagée

Fichiers dans `deploy/` et `infra/` ; avertissement quand un service demande un droit qu'aucun flux n'utilise.

## Autres projets regardés

- JulieOps — MIT, Java : topologie Kafka décrite en fichier.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
