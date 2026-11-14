# Registre de schémas, Avro et JSON Schema

| | |
| --- | --- |
| Branche | `feature/registre-de-schemas` |
| Famille | Événements |
| État | Identifiée, non implémentée |
| Référence open source | [Apicurio Registry](https://github.com/Apicurio/apicurio-registry) |
| Licence | Apache-2.0 |
| Langage d'origine | Java |
| Portage vers Rust | **Crate Rust disponible** |

## La feature

Enregistrer les schémas des topics dans un registre et sérialiser en Avro ou JSON Schema en plus de protobuf.

## Pourquoi c'est pertinent pour Service Builder

Point 4 de la feuille de route. Nécessaire pour s'insérer dans un parc Kafka existant.

## Portage vers Rust

Rien à porter du registre : on l'appelle. Côté Rust, `schema_registry_converter` et `apache-avro` couvrent le format de fil.

## Intégration envisagée

Choix du format par topic, enregistrement des schémas à l'export, service registre dans `docker-compose.yml`.

## Autres projets regardés

- Karapace — Apache-2.0, Python, compatible avec l'API Confluent.
- Confluent Schema Registry — écarté : Confluent Community License.

## Vérification

Licence lue dans le fichier de licence du dépôt de référence le 2026-11-14 ; à relire avant de reprendre du code, les licences changent. L'intégration envisagée est une première piste, pas une conception.
