# Clés d'API émises aux clients

| | |
| --- | --- |
| Branche | `feature/cles-api` |
| Famille | Authentification |
| État | Identifiée, non implémentée |
| Référence open source | [prefixed-api-key](https://github.com/seamapi/prefixed-api-key) |
| Licence | MIT |
| Langage d'origine | TypeScript |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Créer, lister, révoquer et faire tourner des clés d'API : préfixe lisible, secret haché, portées, expiration, dernière utilisation.

## Pourquoi c'est pertinent pour Service Builder

`apiKey` n'existe que pour les appels sortants ; rien pour authentifier un client machine sur les routes générées.

## Portage vers Rust

Très petite réécriture ; hachage et jetons aléatoires sont déjà dans `runtime/crypto.rs`.

## Intégration envisagée

Mode d'authentification `apiKey` d'une route, table `api_keys`, étapes `ApiKeyCreate` et `ApiKeyRevoke`.

## Autres projets regardés

- Unkey — licence non vérifiée (fichier introuvable) : non retenu.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
