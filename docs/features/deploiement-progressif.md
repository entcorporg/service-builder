# Déploiement progressif

| | |
| --- | --- |
| Branche | `feature/deploiement-progressif` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Argo Rollouts](https://github.com/argoproj/argo-rollouts) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Envoyer une nouvelle version à 5 % du trafic, comparer taux d'erreur et latence, puis poursuivre ou revenir en arrière automatiquement.

## Pourquoi c'est pertinent pour Service Builder

Suite logique des fiches Kubernetes et *Tableaux de bord et alertes* : les mesures à surveiller sont déjà connues du générateur.

## Portage vers Rust

Rien à porter : générer les ressources `Rollout` et `AnalysisTemplate`.

## Intégration envisagée

Stratégie de déploiement par service dans les réglages du projet.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
