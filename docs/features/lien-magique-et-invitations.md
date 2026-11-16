# Lien magique et invitations

| | |
| --- | --- |
| Branche | `feature/lien-magique-et-invitations` |
| Famille | Authentification |
| État | Identifiée, non implémentée |
| Référence open source | [Supabase Auth](https://github.com/supabase/auth) |
| Licence | MIT |
| Langage d'origine | Go |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Connexion par lien à usage unique envoyé par mail, et invitation d'un utilisateur dans un tenant avec un rôle.

## Pourquoi c'est pertinent pour Service Builder

`RandomToken` et le service mail existent : il manque le modèle (jeton haché, expiration, usage unique) et les routes.

## Portage vers Rust

Petite réécriture, essentiellement un assemblage de blocs existants et un modèle de mail.

## Intégration envisagée

Modèle de flux prêt à l'emploi + table `one_time_tokens`, plutôt qu'une nouvelle étape.

## Autres projets regardés

- Ory Kratos — Apache-2.0, Go.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
