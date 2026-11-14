# Notifications multicanal et préférences

| | |
| --- | --- |
| Branche | `feature/notifications-et-preferences` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [Novu](https://github.com/novuhq/novu) |
| Licence | MIT (cœur ; dossier `enterprise/` sous licence commerciale) |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Un envoi « notifier » qui choisit le canal (mail, SMS, push, in-app) selon les préférences de la personne, avec désabonnement et regroupement en digest.

## Pourquoi c'est pertinent pour Service Builder

Le service mail gère rebonds et suppressions ; il manque les préférences, `List-Unsubscribe` et les autres canaux.

## Portage vers Rust

Réécriture modérée : modèle abonné / préférence / workflow de notification, fenêtre de digest.

## Intégration envisagée

Table `notification_preferences`, étape `Notify { template, to }`, route de désabonnement en un clic.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
