# Robustesse des mots de passe

| | |
| --- | --- |
| Branche | `feature/robustesse-des-mots-de-passe` |
| Famille | Authentification |
| État | Identifiée, non implémentée |
| Référence open source | [zxcvbn (portage Rust `zxcvbn-rs`)](https://github.com/shssoichiro/zxcvbn-rs) |
| Licence | MIT |
| Langage d'origine | Rust (original Dropbox en CoffeeScript, MIT) |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Estimer la force d'un mot de passe et refuser ceux qui figurent dans des fuites connues (interrogation par préfixe de hachage, sans divulguer le mot de passe).

## Pourquoi c'est pertinent pour Service Builder

`HashPassword` existe, mais rien ne juge le mot de passe avant de le hacher.

## Portage vers Rust

Dépendance directe ; la vérification de fuite est un `httpCall` vers une API de plages de hachage.

## Intégration envisagée

Étape `PasswordStrength { password, bind }` et option « refuser les mots de passe compromis ».

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
