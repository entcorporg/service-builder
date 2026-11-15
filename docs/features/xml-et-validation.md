# XML : lecture, écriture et validation

| | |
| --- | --- |
| Branche | `feature/xml-et-validation` |
| Famille | Données |
| État | Identifiée, non implémentée |
| Référence open source | [quick-xml](https://github.com/tafia/quick-xml) |
| Licence | MIT |
| Langage d'origine | Rust |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Lire et produire du XML à partir des schémas du projet, et valider un document contre un XSD et des règles Schematron.

## Pourquoi c'est pertinent pour Service Builder

Tout le modèle est en JSON et protobuf (presque aucune trace de XML dans le dépôt). Or facturation électronique, banque (ISO 20022), santé et administrations échangent en XML.

## Portage vers Rust

Lecture et écriture : `quick-xml` avec `serde`. Validation XSD : liaison vers libxml2. Schematron : pas d'équivalent Rust mûr, la référence est ph-schematron (Java) ; prévoir un service de validation à part ou une traduction des règles.

## Intégration envisagée

Format XML sur un schéma (noms d'éléments, espaces de noms), étapes `XmlParse`, `XmlRender`, `XmlValidate`.

## Autres projets regardés

- ph-schematron — Apache-2.0, Java : validation Schematron de référence.
- phive — Apache-2.0, Java : moteur de validation de documents métier.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
