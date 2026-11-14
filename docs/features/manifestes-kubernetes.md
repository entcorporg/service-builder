# Manifestes Kubernetes et chart Helm

| | |
| --- | --- |
| Branche | `feature/manifestes-kubernetes` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [Kompose](https://github.com/kubernetes/kompose) |
| Licence | Apache-2.0 |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Générer déploiements, services, sondes, secrets et mise à l'échelle à côté du `docker-compose.yml`.

## Pourquoi c'est pertinent pour Service Builder

Aucune trace dans le dépôt ; c'est la marche entre « ça tourne chez moi » et la production.

## Portage vers Rust

Petite réécriture : un générateur de YAML de plus ; Kompose sert de référence pour la correspondance compose → Kubernetes.

## Intégration envisagée

Dossier `deploy/` dans l'export, valeurs par environnement.

## Autres projets regardés

- Encore — MPL-2.0, Go et Rust : génère l'infrastructure depuis le modèle applicatif.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
