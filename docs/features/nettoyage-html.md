# Nettoyage de HTML

| | |
| --- | --- |
| Branche | `feature/nettoyage-html` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [Ammonia](https://github.com/rust-ammonia/ammonia) |
| Licence | MIT ou Apache-2.0 |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Retirer scripts et attributs dangereux d'un HTML saisi par un utilisateur, selon une liste blanche.

## Pourquoi c'est pertinent pour Service Builder

Nécessaire dès qu'un champ de texte riche est stocké puis réaffiché ou envoyé par mail.

## Portage vers Rust

Dépendance directe.

## Intégration envisagée

Fonction d'expression `sanitizeHtml(...)` ou règle de validation sur un champ.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
