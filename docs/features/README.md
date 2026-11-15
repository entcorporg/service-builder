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
| Nettoyage de HTML | `feature/nettoyage-html` | [Ammonia](https://github.com/rust-ammonia/ammonia) | MIT ou Apache-2.0 | Rust | Déjà en Rust |
| Analyse antivirus des fichiers | `feature/analyse-antivirus` | [ClamAV](https://github.com/Cisco-Talos/clamav) | GPL-2.0 | C | Réécriture à prévoir |
| Appel de modèle de langage | `feature/appel-de-modele-de-langage` | [Rig](https://github.com/0xPlaygrounds/rig) | MIT | Rust | Déjà en Rust |

### Service

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Webhooks sortants | `feature/webhooks-sortants` | [Svix](https://github.com/svix/svix-webhooks) | MIT | Rust | Déjà en Rust |
| Notifications multicanal et préférences | `feature/notifications-et-preferences` | [Novu](https://github.com/novuhq/novu) | MIT | TypeScript | Réécriture à prévoir |
| Traductions des mails et des messages | `feature/traductions` | [Fluent](https://github.com/projectfluent/fluent-rs) | MIT ou Apache-2.0 | Rust | Déjà en Rust |
| Paiements | `feature/paiements` | [Hyperswitch](https://github.com/juspay/hyperswitch) | Apache-2.0 | Rust | Déjà en Rust |
| Facture électronique (EN 16931 : Factur-X, UBL, CII) | `feature/facture-electronique` | [Mustang Project](https://github.com/ZUGFeRD/mustangproject) | Apache-2.0 | Java | Réécriture à prévoir |
| Point d'accès Peppol (AS4) | `feature/point-d-acces-peppol` | [phase4](https://github.com/phax/phase4) | Apache-2.0 | Java | Réécriture à prévoir |
| Archivage à valeur probante | `feature/archivage-probant` | [DSS (Digital Signature Service de la Commission européenne)](https://github.com/esig/dss) | LGPL-2.1 | Java | Crate Rust disponible |
| Messagerie entre utilisateurs | `feature/messagerie-entre-utilisateurs` | [Chatwoot](https://github.com/chatwoot/chatwoot) | MIT | Ruby | Réécriture à prévoir |
| Liens de suivi et attribution | `feature/liens-de-suivi` | [Kutt](https://github.com/thedevs-network/kutt) | MIT | TypeScript | Réécriture à prévoir |

### Moteur

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Étapes durables et sagas | `feature/workflows-durables` | [Temporal](https://github.com/temporalio/temporal) | MIT | Go | Réécriture à prévoir |

### Option de table

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Verrou optimiste et requêtes conditionnelles | `feature/verrou-optimiste` | [Ecto (`optimistic_lock`)](https://github.com/elixir-ecto/ecto) | Apache-2.0 | Elixir | Réécriture à prévoir |
| Isolation des tenants garantie par la base | `feature/isolation-des-tenants-en-base` | [Supabase (sécurité par ligne)](https://github.com/supabase/supabase) | Apache-2.0 | TypeScript | Réécriture à prévoir |

### Option d'étape

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Disjoncteur et cloisonnement des appels sortants | `feature/disjoncteur` | [Resilience4j](https://github.com/resilience4j/resilience4j) | Apache-2.0 | Java | Crate Rust disponible |
| Corps JSON imbriqués pour les appels HTTP | `feature/corps-json-imbrique` | [JSONata](https://github.com/jsonata-js/jsonata) | MIT | JavaScript | Réécriture à prévoir |
| Limitation de débit partagée entre instances | `feature/limitation-de-debit-partagee` | [redis-cell](https://github.com/brandur/redis-cell) | MIT | Rust | Déjà en Rust |

### Bloc SQL

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Recherche plein texte | `feature/recherche-plein-texte` | [PostgREST (opérateurs `fts`, `plfts`, `phfts`, `wfts`)](https://github.com/PostgREST/postgrest) | MIT | Haskell | Réécriture à prévoir |
| Agrégats et auto-jointures dans le bloc SQL | `feature/agregats-sql` | [PostgREST (fonctions d'agrégat)](https://github.com/PostgREST/postgrest) | MIT | Haskell | Réécriture à prévoir |
| Conseil d'index à l'édition | `feature/conseil-d-index` | [index_advisor](https://github.com/supabase/index_advisor) | Licence PostgreSQL | PL/pgSQL | Réécriture à prévoir |

### Authentification

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Connexion OAuth 2 / OpenID Connect | `feature/connexion-oidc` | [Ory Kratos](https://github.com/ory/kratos) | Apache-2.0 | Go | Crate Rust disponible |
| Lien magique et invitations | `feature/lien-magique-et-invitations` | [Supabase Auth](https://github.com/supabase/auth) | MIT | Go | Réécriture à prévoir |
| Politiques d'autorisation | `feature/politiques-autorisation` | [Cedar](https://github.com/cedar-policy/cedar) | Apache-2.0 | Rust | Déjà en Rust |
| Clés d'API émises aux clients | `feature/cles-api` | [prefixed-api-key](https://github.com/seamapi/prefixed-api-key) | MIT | TypeScript | Réécriture à prévoir |
| SSO SAML et provisionnement SCIM | `feature/sso-saml-et-scim` | [Ory Polis (ex-BoxyHQ SAML Jackson)](https://github.com/ory/polis) | Apache-2.0 | TypeScript | Crate Rust disponible |
| Protection contre les robots | `feature/protection-anti-robots` | [ALTCHA](https://github.com/altcha-org/altcha) | MIT | TypeScript | Crate Rust disponible |
| Robustesse des mots de passe | `feature/robustesse-des-mots-de-passe` | [zxcvbn (portage Rust `zxcvbn-rs`)](https://github.com/shssoichiro/zxcvbn-rs) | MIT | Rust | Déjà en Rust |
| Connexions aux comptes tiers des utilisateurs | `feature/connexions-a-des-api-tierces` | [oauth2 (crate)](https://github.com/ramosbugs/oauth2-rs) | MIT ou Apache-2.0 | Rust | Déjà en Rust |

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
| Requête-réponse sur Kafka | `feature/requete-reponse-kafka` | [Spring Kafka (`ReplyingKafkaTemplate`)](https://github.com/spring-projects/spring-kafka) | Apache-2.0 | Java | Réécriture à prévoir |
| Enveloppe CloudEvents | `feature/cloudevents` | [CloudEvents SDK pour Rust](https://github.com/cloudevents/sdk-rust) | Apache-2.0 | Rust | Déjà en Rust |
| Charges utiles volumineuses (claim check) | `feature/claim-check` | [Apache Camel (EIP Claim Check)](https://github.com/apache/camel) | Apache-2.0 | Java | Réécriture à prévoir |
| Routage, découpage et regroupement de messages | `feature/routage-de-messages` | [Apache Camel](https://github.com/apache/camel) | Apache-2.0 | Java | Réécriture à prévoir |
| Connecteurs sources et destinations | `feature/connecteurs` | [Bento](https://github.com/warpstreamlabs/bento) | MIT | Go | Réécriture à prévoir |
| Chiffrement des événements par sujet | `feature/chiffrement-par-sujet` | [Kroxylicious (chiffrement d'enregistrements)](https://github.com/kroxylicious/kroxylicious) | Apache-2.0 | Java | Réécriture à prévoir |
| Consommation parallèle et par lots | `feature/consommation-parallele` | [Confluent Parallel Consumer](https://github.com/confluentinc/parallel-consumer) | Apache-2.0 | Java | Réécriture à prévoir |
| Montée de version des événements | `feature/versions-d-evenements` | [Axon Framework (upcasters)](https://github.com/AxonFramework/AxonFramework) | Apache-2.0 | Java | Réécriture à prévoir |

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
| Versionnement d'API | `feature/versionnement-d-api` | [Cadwyn](https://github.com/zmievsa/cadwyn) | MIT | Python | Réécriture à prévoir |
| Serveur MCP du backend généré | `feature/serveur-mcp-genere` | [SDK Rust du Model Context Protocol](https://github.com/modelcontextprotocol/rust-sdk) | En transition de MIT vers Apache-2.0 | Rust | Déjà en Rust |
| Collection de requêtes exportée | `feature/collection-de-requetes` | [Bruno](https://github.com/usebruno/bruno) | MIT | JavaScript | Réécriture à prévoir |
| Clients dans d'autres langages | `feature/clients-multi-langages` | [OpenAPI Generator](https://github.com/OpenAPITools/openapi-generator) | Apache-2.0 | Java | Réécriture à prévoir |

### Données

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Recherche vectorielle | `feature/recherche-vectorielle` | [pgvector](https://github.com/pgvector/pgvector) | Licence PostgreSQL | C | Crate Rust disponible |
| Requêtes géographiques | `feature/requetes-geographiques` | [PostGIS](https://github.com/postgis/postgis) | GPL-2.0 | C / C++ | Crate Rust disponible |
| Fuseaux horaires et types calendaires | `feature/fuseaux-horaires` | [Jiff](https://github.com/BurntSushi/jiff) | MIT ou Unlicense | Rust | Déjà en Rust |
| Import et export CSV | `feature/import-export-csv` | [PostgREST (`Accept: text/csv`, insertion en masse)](https://github.com/PostgREST/postgrest) | MIT | Haskell | Crate Rust disponible |
| Effacement et export RGPD | `feature/effacement-rgpd` | [Fides](https://github.com/ethyca/fides) | Apache-2.0 | Python | Réécriture à prévoir |
| Cache de lecture | `feature/cache-de-lecture` | [Moka](https://github.com/moka-rs/moka) | MIT ou Apache-2.0 | Rust | Déjà en Rust |
| Partitionnement et rétention des tables | `feature/partitionnement-et-retention` | [pg_partman](https://github.com/pgpartman/pg_partman) | Licence PostgreSQL | PL/pgSQL et C | Réécriture à prévoir |
| Identifiants : UUIDv7, identifiants courts, numéros sans trou | `feature/identifiants` | [Sqids](https://github.com/sqids/sqids-rust) | MIT | Rust | Déjà en Rust |
| Migrations sans interruption de service | `feature/migrations-sans-interruption` | [pgroll](https://github.com/xataio/pgroll) | Apache-2.0 | Go | Crate Rust disponible |
| Copie anonymisée des données | `feature/donnees-anonymisees` | [Greenmask](https://github.com/GreenmaskIO/greenmask) | Apache-2.0 | Go | Réécriture à prévoir |
| XML : lecture, écriture et validation | `feature/xml-et-validation` | [quick-xml](https://github.com/tafia/quick-xml) | MIT | Rust | Crate Rust disponible |

### Exploitation

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Traçage OpenTelemetry | `feature/tracage-opentelemetry` | [OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust) | Apache-2.0 | Rust | Déjà en Rust |
| Tests d'intégration générés | `feature/tests-integration` | [Testcontainers pour Rust](https://github.com/testcontainers/testcontainers-rs) | MIT ou Apache-2.0 | Rust | Déjà en Rust |
| Tests de contrat entre services | `feature/tests-de-contrat` | [Pact (implémentation de référence)](https://github.com/pact-foundation/pact-reference) | MIT | Rust | Déjà en Rust |
| Manifestes Kubernetes et chart Helm | `feature/manifestes-kubernetes` | [Kompose](https://github.com/kubernetes/kompose) | Apache-2.0 | Go | Réécriture à prévoir |
| Tableaux de bord et alertes générés | `feature/tableaux-de-bord-et-alertes` | [Sloth](https://github.com/slok/sloth) | Apache-2.0 | Go | Réécriture à prévoir |
| Tests de charge générés | `feature/tests-de-charge` | [Goose](https://github.com/tag1consulting/goose) | Apache-2.0 | Rust | Déjà en Rust |
| Injection de pannes | `feature/injection-de-pannes` | [Toxiproxy](https://github.com/Shopify/toxiproxy) | MIT | Go | Réécriture à prévoir |
| Tests de robustesse et d'isolation générés | `feature/tests-de-fuzzing-et-d-isolation` | [Schemathesis](https://github.com/schemathesis/schemathesis) | MIT | Python | Réécriture à prévoir |
| Documentation d'architecture générée | `feature/documentation-d-architecture` | [Mermaid](https://github.com/mermaid-js/mermaid) | MIT | JavaScript | Réécriture à prévoir |
| Infrastructure générée (OpenTofu) | `feature/infrastructure-opentofu` | [OpenTofu](https://github.com/opentofu/opentofu) | MPL-2.0 | Go | Réécriture à prévoir |
| Audit des dépendances et inventaire logiciel | `feature/audit-des-dependances-et-sbom` | [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) | MIT ou Apache-2.0 | Rust | Déjà en Rust |
| Sauvegardes et restauration | `feature/sauvegardes-et-restauration` | [pgBackRest](https://github.com/pgbackrest/pgbackrest) | MIT | C | Réécriture à prévoir |
| Droits minimaux générés | `feature/droits-minimaux` | [Strimzi (ressources `KafkaTopic` et `KafkaUser`)](https://github.com/strimzi/strimzi-kafka-operator) | Apache-2.0 | Java | Réécriture à prévoir |
| Déploiement progressif | `feature/deploiement-progressif` | [Argo Rollouts](https://github.com/argoproj/argo-rollouts) | Apache-2.0 | Go | Réécriture à prévoir |

### Application

| Feature | Branche | Référence | Licence | Langage d'origine | Portage vers Rust |
| --- | --- | --- | --- | --- | --- |
| Importer un existant | `feature/import-d-un-existant` | [SeaSchema](https://github.com/SeaQL/sea-schema) | MIT ou Apache-2.0 | Rust | Déjà en Rust |
| Débogueur de flux | `feature/debogueur-de-flux` | [Node-RED (panneau de débogage)](https://github.com/node-red/node-red) | Apache-2.0 | JavaScript | Réécriture à prévoir |
| Règles de style d'API | `feature/regles-de-style-d-api` | [vacuum](https://github.com/daveshanley/vacuum) | MIT | Go | Réécriture à prévoir |
| Services externes dans le graphe | `feature/services-externes` | [Dapr](https://github.com/dapr/dapr) | Apache-2.0 | Go | Réécriture à prévoir |

## Écarté pour l'instant

- Cœur analytique à la PostHog (ingestion, cohortes, funnels, session replay) : un produit entier, qui demande ClickHouse.
- Moteur de facturation complet à la Lago : un produit entier ; les compteurs d'usage couvrent le besoin de base.
- Interface d'administration générée : un second produit à maintenir.
