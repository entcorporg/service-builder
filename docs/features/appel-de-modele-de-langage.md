# Appel de modèle de langage

| | |
| --- | --- |
| Branche | `feature/appel-de-modele-de-langage` |
| Famille | Bloc de flux |
| État | Identifiée, non implémentée |
| Référence open source | [Rig](https://github.com/0xPlaygrounds/rig) |
| Licence | MIT |
| Langage d'origine | Rust |
| Portage vers Rust | **Déjà en Rust** |

## La feature

Un bloc qui envoie une consigne et des données à un modèle de langage et reçoit une réponse typée, validée contre un schéma du projet.

## Pourquoi c'est pertinent pour Service Builder

Classer, extraire, résumer deviennent des étapes de flux. La sortie structurée s'appuie sur le typage existant ; `httpCall` seul ne valide rien.

## Portage vers Rust

Bibliothèque Rust multi-fournisseurs ; à évaluer face à un simple client HTTP généré, plus léger.

## Intégration envisagée

Étape `Generate { model, prompt, input, outputSchema, bind }`, clé d'API par variable d'environnement, délai et reprises.

## Autres projets regardés

- LiteLLM — MIT hors dossier entreprise, Python : passerelle multi-fournisseurs, comme service à part.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
