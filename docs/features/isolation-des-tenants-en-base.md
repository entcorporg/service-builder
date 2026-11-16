# Isolation des tenants garantie par la base

| | |
| --- | --- |
| Branche | `feature/isolation-des-tenants-en-base` |
| Famille | Option de table |
| État | Identifiée, non implémentée |
| Référence open source | [Supabase (sécurité par ligne)](https://github.com/supabase/supabase) |
| Licence | Apache-2.0 |
| Langage d'origine | TypeScript (les politiques elles-mêmes sont du SQL) |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Générer les clés étrangères composites `(tenant_id, id)` et des politiques de sécurité par ligne PostgreSQL, en plus des filtres des blocs SQL.

## Pourquoi c'est pertinent pour Service Builder

Limite documentée : les clés composites « s'ajoutent à la main », et un bloc *Code Rust* contourne les filtres de tenant. La base devient un second rempart.

## Portage vers Rust

Petite réécriture : émettre `CREATE POLICY` et `SET LOCAL` du tenant en début de transaction ; les migrations existantes portent le reste.

## Intégration envisagée

Case « isolation par la base » sur une table à tenant ; diagnostic si une relation traverse deux tenants.

## Autres projets regardés

- PostgREST — MIT, Haskell : même modèle, le rôle et le tenant passés à la transaction.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
