# Journal d'audit

| | |
| --- | --- |
| Branche | `feature/journal-audit` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [Retraced](https://github.com/retracedhq/retraced) |
| Licence | Apache-2.0 |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Un bloc « Auditer » : qui, quelle action, sur quelle ressource, valeurs avant/après, écrit dans la transaction du flux ; une route de consultation filtrable par tenant.

## Pourquoi c'est pertinent pour Service Builder

Presque rien dans le dépôt alors que tenant et permissions sont partout. Exigé par la plupart des clients B2B.

## Portage vers Rust

Petite réécriture : le modèle d'événement (acteur, action, cible, champs) est simple, l'écriture passe par le bloc SQL existant.

## Intégration envisagée

Table `audit_events` en ajout seul, étape `Audit { action, target, before, after }`, route de lecture paginée par curseur.

## Autres projets regardés

- PostHog activity log — MIT, Python : bon modèle de diff avant/après.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
