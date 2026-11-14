# Analyse antivirus des fichiers

| | |
| --- | --- |
| Branche | `feature/analyse-antivirus` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [ClamAV](https://github.com/Cisco-Talos/clamav) |
| Licence | GPL-2.0 (utilisé comme service à part, sans copie de code : pas d'effet sur le code généré) |
| Langage d'origine | C |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Analyser un fichier envoyé avant de le rendre disponible, et le mettre en quarantaine s'il est infecté.

## Pourquoi c'est pertinent pour Service Builder

Exigence courante dès qu'on accepte des pièces jointes ; complète `ImageSniff` et les blocs objet.

## Portage vers Rust

Rien à porter : le service parle au démon `clamd` par son protocole réseau, un client de quelques dizaines de lignes.

## Intégration envisagée

Étape `ScanFile { object, bind }`, conteneur `clamav` dans `docker-compose.yml`.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
