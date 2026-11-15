# Audit des dépendances et inventaire logiciel

| | |
| --- | --- |
| Branche | `feature/audit-des-dependances-et-sbom` |
| Famille | Exploitation |
| État | Identifiée, non implémentée |
| Référence open source | [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) |
| Licence | MIT ou Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Livrer avec l'export la configuration qui refuse les dépendances vulnérables ou sous licence interdite, et produire l'inventaire des composants (SBOM).

## Pourquoi c'est pertinent pour Service Builder

Demandé dans les questionnaires de sécurité des clients ; le générateur choisit lui-même les dépendances, il peut donc en répondre.

## Portage vers Rust

Outil à lancer tel quel ; on génère son fichier `deny.toml`.

## Intégration envisagée

`deny.toml` et étape de CI dans l'export ; SBOM au format CycloneDX.

## Autres projets regardés

- cyclonedx-rust-cargo — Apache-2.0, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
