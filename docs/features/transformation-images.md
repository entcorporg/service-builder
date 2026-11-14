# Transformation d'images

| | |
| --- | --- |
| Branche | `feature/transformation-images` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [imgproxy](https://github.com/imgproxy/imgproxy) |
| Licence | Apache-2.0 |
| Langage d'origine | Go (au-dessus de libvips, en C) |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Redimensionner, recadrer, convertir et retirer les métadonnées d'une image envoyée.

## Pourquoi c'est pertinent pour Service Builder

`ImageSniff` vérifie le type mais rien ne produit de vignette.

## Portage vers Rust

Reprendre les garde-fous d'imgproxy (limites de dimensions, bombes de décompression) ; le traitement se fait avec la crate `image` (MIT / Apache-2.0).

## Intégration envisagée

Étape `ImageTransform { input, width, height, format, bind }`.

## Autres projets regardés

- image-rs/image — MIT / Apache-2.0, Rust.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
