# Génération de PDF

| | |
| --- | --- |
| Branche | `feature/generation-pdf` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [Typst](https://github.com/typst/typst) |
| Licence | Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Produire un PDF (facture, attestation, bon de livraison) depuis un modèle et des données du flux, puis le stocker.

## Pourquoi c'est pertinent pour Service Builder

Complète les blocs objet (S3) et l'éditeur de modèles de mail.

## Portage vers Rust

Le compilateur s'utilise comme bibliothèque Rust, sans navigateur sans tête ni binaire externe.

## Intégration envisagée

Modèles `.typ` dans le projet, étape `RenderPdf { template, data, bind }`.

## Autres projets regardés

- Gotenberg — MIT, Go : HTML vers PDF par Chromium, un conteneur de plus.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
