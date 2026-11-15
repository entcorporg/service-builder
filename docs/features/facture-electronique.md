# Facture électronique (EN 16931 : Factur-X, UBL, CII)

| | |
| --- | --- |
| Branche | `feature/facture-electronique` |
| Famille | Service |
| État | Identifiée, non implémentée |
| Référence open source | [Mustang Project](https://github.com/ZUGFeRD/mustangproject) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Réécriture à prévoir** |

## La feature

Produire, lire et valider une facture aux formats de la norme européenne : Factur-X (PDF/A-3 avec XML embarqué), UBL et CII, avec les règles de gestion EN 16931 et les statuts de cycle de vie.

## Pourquoi c'est pertinent pour Service Builder

Socle de toute plateforme de facturation en Europe (réforme française, XRechnung, Peppol). Dépend de la fiche *XML : lecture, écriture et validation*.

## Portage vers Rust

Grosse réécriture si tout est fait en Rust (modèle sémantique, trois syntaxes, PDF/A-3, plusieurs centaines de règles). Plus réaliste au départ : Mustang comme service à part, appelé depuis les flux.

## Intégration envisagée

Modèle de service « factures » : schéma de facture fourni, étapes `InvoiceRender`, `InvoiceParse`, `InvoiceValidate` ; cycle de vie par la fiche *Machine à états*.

## Autres projets regardés

- GOBL — Apache-2.0, Go : modèle de document commercial avec conversions vers les formats nationaux.
- Règles de validation EN 16931 (ConnectingEurope) — EUPL-1.2 : artefacts Schematron officiels, à utiliser tels quels.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
