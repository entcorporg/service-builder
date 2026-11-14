# Disjoncteur et cloisonnement des appels sortants

| | |
| --- | --- |
| Branche | `feature/disjoncteur` |
| Famille | Option d'étape |
| État | Identifiée, non implémentée |
| Référence open source | [Resilience4j](https://github.com/resilience4j/resilience4j) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Couper les appels vers une dépendance qui échoue, puis réessayer après un délai ; limiter le nombre d'appels simultanés.

## Pourquoi c'est pertinent pour Service Builder

`httpCall` et `CallRpc` ont des reprises mais rien n'empêche un service lent d'en entraîner d'autres.

## Portage vers Rust

Le modèle (fermé / ouvert / semi-ouvert, fenêtre glissante) vient de Resilience4j ; l'implémentation peut partir de `failsafe-rs` (MIT) ou des couches `tower`.

## Intégration envisagée

Options `circuit` et `maxConcurrent` sur `httpCall` et `CallRpc`, état exposé dans `/metrics`.

## Autres projets regardés

- dmexe/failsafe-rs — MIT, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
