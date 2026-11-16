# Infrastructure générée (OpenTofu)

| | |
| --- | --- |
| Branche | `feature/infrastructure-opentofu` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [OpenTofu](https://github.com/opentofu/opentofu) |
| Licence | MPL-2.0 (on génère des fichiers de configuration, on ne reprend pas de code) |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Décrire l'infrastructure gérée dont le projet a besoin : cluster Kafka, PostgreSQL, stockage objet, topics, bases et secrets.

## Pourquoi c'est pertinent pour Service Builder

Complète la fiche *Manifestes Kubernetes* : celle-ci déploie les services, celle-là crée ce dont ils dépendent.

## Portage vers Rust

Rien à porter : émettre des fichiers `.tf`. Le travail est dans le choix d'un ou deux hébergeurs cibles.

## Intégration envisagée

Dossier `infra/` dans l'export, un module par hébergeur.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
