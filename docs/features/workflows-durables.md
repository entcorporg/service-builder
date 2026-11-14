# Étapes durables et sagas

| | |
| --- | --- |
| Branche | `feature/workflows-durables` |
| Famille | Moteur |
| État | Identifiée, non implémentée |
| Référence open source | [Temporal](https://github.com/temporalio/temporal) |
| Licence | MIT |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

« Attendre une durée », « Attendre un événement », reprise après redémarrage, et compensation (saga) quand une étape échoue.

## Pourquoi c'est pertinent pour Service Builder

Point 2 de la feuille de route. C'est ce qui sépare un enchaînement de handlers d'un processus métier long (paiement, onboarding, relance).

## Portage vers Rust

Ne pas porter Temporal : reprendre le modèle (historique d'événements, étapes rejouées, minuteries) dans une table PostgreSQL. C'est la plus grosse réécriture de la liste.

## Intégration envisagée

Table `flow_runs` + `flow_steps`, étapes `Sleep`, `WaitForEvent`, `Compensate`, reprise par le worker de jobs.

## Autres projets regardés

- DBOS Transact — MIT, TypeScript : le plus proche de la cible (durabilité dans PostgreSQL, sans serveur à part).
- Hatchet — MIT, Go.
- Inngest (SSPL) et Restate (BSL 1.1) — écartés comme sources.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
