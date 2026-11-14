# Features à venir

Une fiche par feature identifiée pour faire de Service Builder le générateur de backends événementiels le plus complet,
en s'inspirant de ce que l'open source fait de mieux. Chaque fiche vit sur **sa propre branche** `feature/<nom>`, partie
de `main`, dans `docs/features/<nom>.md` : l'implémentation se fera sur cette branche, à côté de sa fiche.

## Convention pour une nouvelle feature

1. Créer `feature/<nom>` depuis `main` (nom en français, minuscules, tirets).
2. Y écrire `docs/features/<nom>.md` avec les mêmes rubriques : la feature, pourquoi elle est pertinente, la référence
   open source, sa licence, son langage d'origine, le portage vers Rust, l'intégration envisagée.
3. Ajouter sa ligne au tableau ci-dessous.

Une référence doit avoir une licence qui autorise la lecture et la réutilisation du code (MIT, Apache-2.0, BSD, MPL…).
Les projets sous AGPL, SSPL ou BSL ne sont cités que comme projets écartés. Le portage vers Rust prend trois valeurs :

- **Déjà en Rust** : la référence est en Rust, elle se lit ou s'utilise comme dépendance.
- **Crate Rust disponible** : la référence est dans un autre langage, mais une crate couvre l'essentiel ; seule
  l'intégration est à écrire.
- **Réécriture à prévoir** : la logique est à réécrire en Rust à partir de la référence.

## Les fiches

### Bloc de flux

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Drapeaux de fonctionnalité | `feature/drapeaux-de-fonctionnalite` | [PostHog (service `rust/feature-flags`)](https://github.com/PostHog/posthog/tree/master/rust/feature-flags) | MIT | Rust | Déjà en Rust |
| Journal d'audit | `feature/journal-audit` | [Retraced](https://github.com/retracedhq/retraced) | Apache-2.0 | TypeScript | Réécriture à prévoir |
| Compteurs d'usage et quotas | `feature/quotas-et-usage` | [OpenMeter](https://github.com/openmeterio/openmeter) | Apache-2.0 | Go | Réécriture à prévoir |
| Machine à états | `feature/machine-a-etats` | [XState](https://github.com/statelyai/xstate) | MIT | TypeScript | Réécriture à prévoir |
| Règles métier et tables de décision | `feature/regles-metier` | [GoRules Zen](https://github.com/gorules/zen) | MIT | Rust | Déjà en Rust |
| Génération de PDF | `feature/generation-pdf` | [Typst](https://github.com/typst/typst) | Apache-2.0 | Rust | Déjà en Rust |
| Transformation d'images | `feature/transformation-images` | [imgproxy](https://github.com/imgproxy/imgproxy) | Apache-2.0 | Go | Crate Rust disponible |

### Service

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Webhooks sortants | `feature/webhooks-sortants` | [Svix](https://github.com/svix/svix-webhooks) | MIT | Rust | Déjà en Rust |
| Notifications multicanal et préférences | `feature/notifications-et-preferences` | [Novu](https://github.com/novuhq/novu) | MIT | TypeScript | Réécriture à prévoir |
| Traductions des mails et des messages | `feature/traductions` | [Fluent](https://github.com/projectfluent/fluent-rs) | MIT ou Apache-2.0 | Rust | Déjà en Rust |

### Moteur

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Étapes durables et sagas | `feature/workflows-durables` | [Temporal](https://github.com/temporalio/temporal) | MIT | Go | Réécriture à prévoir |

### Option de table

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Verrou optimiste et requêtes conditionnelles | `feature/verrou-optimiste` | [Ecto (`optimistic_lock`)](https://github.com/elixir-ecto/ecto) | Apache-2.0 | Elixir | Réécriture à prévoir |

### Option d'étape

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Disjoncteur et cloisonnement des appels sortants | `feature/disjoncteur` | [Resilience4j](https://github.com/resilience4j/resilience4j) | Apache-2.0 | Java | Crate Rust disponible |

### Bloc SQL

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Recherche plein texte | `feature/recherche-plein-texte` | [PostgREST (opérateurs `fts`, `plfts`, `phfts`, `wfts`)](https://github.com/PostgREST/postgrest) | MIT | Haskell | Réécriture à prévoir |

### Authentification

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Connexion OAuth 2 / OpenID Connect | `feature/connexion-oidc` | [Ory Kratos](https://github.com/ory/kratos) | Apache-2.0 | Go | Crate Rust disponible |
| Lien magique et invitations | `feature/lien-magique-et-invitations` | [Supabase Auth](https://github.com/supabase/auth) | MIT | Go | Réécriture à prévoir |
| Politiques d'autorisation | `feature/politiques-autorisation` | [Cedar](https://github.com/cedar-policy/cedar) | Apache-2.0 | Rust | Déjà en Rust |

### Événements

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Event sourcing | `feature/event-sourcing` | [Marten](https://github.com/JasperFx/marten) | MIT | C# | Crate Rust disponible |
| Projections de lecture (CQRS) | `feature/projections-cqrs` | [Marten (projections)](https://github.com/JasperFx/marten) | MIT | C# | Réécriture à prévoir |
| Registre de schémas, Avro et JSON Schema | `feature/registre-de-schemas` | [Apicurio Registry](https://github.com/Apicurio/apicurio-registry) | Apache-2.0 | Java | Crate Rust disponible |
| Détection des changements incompatibles | `feature/changements-incompatibles` | [Buf (`buf breaking`)](https://github.com/bufbuild/buf) | Apache-2.0 | Go | Réécriture à prévoir |
| Capture des changements (CDC) | `feature/capture-de-changements` | [Debezium](https://github.com/debezium/debezium) | Apache-2.0 | Java | Crate Rust disponible |
| Reprises différées et messages retardés | `feature/reprises-differees` | [Spring Kafka (retry topics non bloquants)](https://github.com/spring-projects/spring-kafka) | Apache-2.0 | Java | Réécriture à prévoir |
| Rejouer une DLQ ou un topic | `feature/rejeu-dlq` | [Kafbat UI](https://github.com/kafbat/kafka-ui) | Apache-2.0 | Java | Réécriture à prévoir |
| Agrégations par fenêtre et jointures de flux | `feature/agregations-par-fenetre` | [Kafka Streams](https://github.com/apache/kafka) | Apache-2.0 | Java | Réécriture à prévoir |
| Autres brokers : NATS JetStream, RabbitMQ | `feature/autres-brokers` | [Watermill](https://github.com/ThreeDotsLabs/watermill) | MIT | Go | Crate Rust disponible |
| File de tâches en arrière-plan | `feature/file-de-taches` | [River](https://github.com/riverqueue/river) | MPL-2.0 | Go | Crate Rust disponible |
| Tâches planifiées par expression cron | `feature/taches-cron` | [Oban (plugin Cron)](https://github.com/oban-bg/oban) | Apache-2.0 | Elixir | Crate Rust disponible |

### Surface d'API

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| API GraphQL générée | `feature/graphql` | [PostGraphile (dépôt `graphile/crystal`)](https://github.com/graphile/crystal) | MIT | TypeScript | Crate Rust disponible |
| WebSocket bidirectionnel | `feature/websocket` | [Centrifugo](https://github.com/centrifugal/centrifugo) | Apache-2.0 | Go | Réécriture à prévoir |
| gRPC-Web et réflexion gRPC | `feature/grpc-web-et-reflexion` | [Connect](https://github.com/connectrpc/connect-go) | Apache-2.0 | Go | Crate Rust disponible |
| Document AsyncAPI des topics | `feature/asyncapi` | [AsyncAPI (spécification)](https://github.com/asyncapi/spec) | Apache-2.0 | Spécification | Réécriture à prévoir |
| Client TypeScript typé | `feature/client-typescript` | [openapi-typescript](https://github.com/openapi-ts/openapi-typescript) | MIT | TypeScript | Réécriture à prévoir |
| Compression des réponses HTTP | `feature/compression-http` | [tower-http](https://github.com/tower-rs/tower-http) | MIT | Rust | Déjà en Rust |
| Serveur simulé depuis OpenAPI | `feature/serveur-simule` | [Prism](https://github.com/stoplightio/prism) | Apache-2.0 | TypeScript | Réécriture à prévoir |
| Envois de fichiers reprenables | `feature/envois-reprenables` | [tusd](https://github.com/tus/tusd) | MIT | Go | Réécriture à prévoir |

### Données

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Recherche vectorielle | `feature/recherche-vectorielle` | [pgvector](https://github.com/pgvector/pgvector) | Licence PostgreSQL | C | Crate Rust disponible |
| Requêtes géographiques | `feature/requetes-geographiques` | [PostGIS](https://github.com/postgis/postgis) | GPL-2.0 | C / C++ | Crate Rust disponible |
| Fuseaux horaires et types calendaires | `feature/fuseaux-horaires` | [Jiff](https://github.com/BurntSushi/jiff) | MIT ou Unlicense | Rust | Déjà en Rust |
| Import et export CSV | `feature/import-export-csv` | [PostgREST (`Accept: text/csv`, insertion en masse)](https://github.com/PostgREST/postgrest) | MIT | Haskell | Crate Rust disponible |
| Effacement et export RGPD | `feature/effacement-rgpd` | [Fides](https://github.com/ethyca/fides) | Apache-2.0 | Python | Réécriture à prévoir |

### Exploitation

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Traçage OpenTelemetry | `feature/tracage-opentelemetry` | [OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust) | Apache-2.0 | Rust | Déjà en Rust |
| Tests d'intégration générés | `feature/tests-integration` | [Testcontainers pour Rust](https://github.com/testcontainers/testcontainers-rs) | MIT ou Apache-2.0 | Rust | Déjà en Rust |
| Tests de contrat entre services | `feature/tests-de-contrat` | [Pact (implémentation de référence)](https://github.com/pact-foundation/pact-reference) | MIT | Rust | Déjà en Rust |
| Manifestes Kubernetes et chart Helm | `feature/manifestes-kubernetes` | [Kompose](https://github.com/kubernetes/kompose) | Apache-2.0 | Go | Réécriture à prévoir |

## Écarté pour l'instant

- Cœur analytique à la PostHog (ingestion, cohortes, funnels, session replay) : un produit entier, qui demande ClickHouse.
- Moteur de facturation complet à la Lago : un produit entier ; les compteurs d'usage couvrent le besoin de base.
- Interface d'administration générée : un second produit à maintenir.
