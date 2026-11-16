# Services externes dans le graphe

| | |
| --- | --- |
| Branche | `feature/services-externes` |
| Famille | Application |
| État | Identifiée, non implémentée |
| Référence open source | [Dapr](https://github.com/dapr/dapr) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Déclarer dans le projet un service qui n'est pas généré (un conteneur existant, un service écrit à la main ou dans un autre langage) avec ses topics, ses RPC et son image, pour qu'il apparaisse dans le graphe, la validation, le `docker-compose.yml` et l'onglet Pods.

## Pourquoi c'est pertinent pour Service Builder

C'est l'échappatoire à l'échelle d'un service, comme *Code Rust* l'est à l'échelle d'une étape : elle permet d'intégrer ce qu'il ne faut pas réécrire (point d'accès Peppol, moteur de validation Java, modèle de machine learning).

## Portage vers Rust

Rien à porter : Dapr sert de référence pour décrire un composant par son contrat. Le travail est dans le modèle de projet et la validation des contrats aux frontières.

## Intégration envisagée

Nœud « service externe » : image, variables, ports, topics publiés et consommés, `.proto` importé.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
