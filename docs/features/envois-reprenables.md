# Envois de fichiers reprenables

| | |
| --- | --- |
| Branche | `feature/envois-reprenables` |
| Famille | Surface d'API |
| État | Identifiée, non implémentée |
| Référence open source | [tusd](https://github.com/tus/tusd) |
| Licence | MIT |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Envoyer un gros fichier par morceaux et reprendre après une coupure, selon le protocole tus.

## Pourquoi c'est pertinent pour Service Builder

Les URL présignées couvrent les envois simples ; pas la reprise sur mobile ou réseau instable.

## Portage vers Rust

Réécriture modérée du protocole (création, PATCH par décalage, expiration) au-dessus de l'envoi multipart S3.

## Intégration envisagée

Type de route « envoi reprenable » lié à un bloc objet.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
