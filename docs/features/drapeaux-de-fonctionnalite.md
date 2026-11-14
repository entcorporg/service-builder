# Drapeaux de fonctionnalité

| | |
| --- | --- |
| Branche | `feature/drapeaux-de-fonctionnalite` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [PostHog (service `rust/feature-flags`)](https://github.com/PostHog/posthog/tree/master/rust/feature-flags) |
| Licence | MIT (hors dossier `ee/`, sous licence commerciale) |
| Langage d'origine | Rust (le reste de PostHog est en Python et TypeScript) |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Un bloc « Drapeau » qui renvoie un booléen ou une variante, avec déploiement progressif (pourcentage stable par identifiant) et ciblage par propriétés.

## Pourquoi c'est pertinent pour Service Builder

Aucun équivalent dans le dépôt. Se branche directement sur le bloc *Condition* et permet d'activer une branche d'un flux pour 5 % des comptes, sans redéployer.

## Portage vers Rust

L'évaluation (hachage `clé + identifiant` vers un seau, filtres de propriétés) se lit directement en Rust. À adapter : PostHog lit ses drapeaux dans sa propre base, service-builder les déclarerait dans le projet et une table.

## Intégration envisagée

Table `feature_flags` générée, module `runtime/flags.rs`, étape `Flag { key, subject, bind }`.

## Autres projets regardés

- Unleash/yggdrasil — moteur d'évaluation, MIT, Rust : bonne seconde lecture.
- Unleash (serveur) — écarté comme source : AGPL-3.0.
- OpenFeature — spécification Apache-2.0, utile pour le vocabulaire.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
