# Archivage à valeur probante

| | |
| --- | --- |
| Branche | `feature/archivage-probant` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [DSS (Digital Signature Service de la Commission européenne)](https://github.com/esig/dss) |
| Licence | LGPL-2.1 (utilisable comme bibliothèque ou service ; ne pas en copier le code dans le runtime) |
| Langage d'origine | Java |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Conserver un document de façon vérifiable pendant des années : empreinte, chaînage des empreintes, cachet électronique, horodatage qualifié (RFC 3161), stockage non modifiable, preuve restituable.

## Pourquoi c'est pertinent pour Service Builder

Exigé pour les factures, contrats et bulletins. Les blocs objet et le hachage existent ; pas le chaînage, le cachet ni l'horodatage.

## Portage vers Rust

Empreintes et chaînage : immédiat en Rust. Cachet CMS/CAdES et jeton d'horodatage : crates RustCrypto (`cms`, `x509-cert`, MIT / Apache-2.0). Formats PAdES/XAdES complets : DSS comme service à part.

## Intégration envisagée

Étapes `ArchivePut { object }` et `ArchiveProof { id }`, verrouillage des objets côté stockage, job de vérification périodique.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
