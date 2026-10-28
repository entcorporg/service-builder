# Service Builder

Un éditeur visuel, « à la Elementor », pour construire des **backends Rust événementiels** :
vous dessinez des services, des topics Kafka, des schémas protobuf et la logique de chaque
handler avec des blocs ; l'application génère un workspace Cargo complet
(**gRPC** avec tonic pour les RPC, **Kafka** avec rdkafka pour les événements).

Application de bureau Linux : **Tauri 2** (Rust) + **React** (xyflow pour le canvas).

![Architecture et inspecteur](docs/screenshots/architecture.png)

## Le principe

| Elementor | Service Builder |
| --- | --- |
| Page | Projet (un fichier `*.sb.json`) |
| Widgets | Étapes : *Publier un événement*, *Appeler un RPC*, *Condition*, *Variable*, *Journal*, *Répondre*, *Code Rust* |
| Canvas | Graphe d'architecture : services ⇄ topics ⇄ services, dérivé du modèle |
| Panneau de réglages | Inspecteur : RPC, consumers, mappings champ à champ typés |
| Aperçu en direct | Onglet « Code généré » : Rust, `.proto`, `Cargo.toml`, `docker-compose.yml` |
| Publier | « Exporter le code » : un workspace Cargo prêt à `cargo run` |

![Code généré](docs/screenshots/generated-code.png)

Ce qui est vérifié en continu pendant que vous éditez (et qui bloque l'export s'il y a une erreur) :

- les références (topic, message, service, RPC), les noms et l'unicité des ports ;
- le **typage des mappings** : `input.customer.name` (string) ne peut pas remplir un `int64` ;
- la portée des variables (une variable définie dans une branche n'existe pas après elle) ;
- la syntaxe du Rust brut tapé dans les blocs *Code Rust* ou *Rust* ;
- des avertissements de topologie (topic que personne ne publie ou ne consomme, RPC sans réponse…).

![Validation](docs/screenshots/validation.png)

## Démarrer

Prérequis : Rust stable, Node 22, pnpm 10. Sous Ubuntu/Debian, les bibliothèques de Tauri :

```sh
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev \
  librsvg2-dev libssl-dev pkg-config build-essential
```

```sh
pnpm install
pnpm tauri dev      # lance l'application avec rechargement à chaud
```

Un projet d'exemple (Orders → Payments → Notifications) est chargé au démarrage.

## Le code généré

« Exporter le code » écrit un workspace Cargo autonome :

```
contracts/      messages + stubs gRPC (générés depuis le .proto, sans protoc grâce à protox) et noms de topics
runtime/        producer / consumer Kafka, création des topics, DLQ (identique pour tous les projets)
services/<nom>/ un binaire par service : serveur gRPC, consumers, flows.rs (la logique dessinée)
docker-compose.yml   un Kafka local (KRaft) sur localhost:9092
```

```sh
cd mon-export
docker compose up -d
cargo run -p orders        # un terminal par service
```

Pour compiler : un toolchain C/C++, `cmake`, `pkg-config` et les en-têtes OpenSSL
(`sudo apt install build-essential cmake pkg-config libssl-dev`).

Garanties de livraison : les consumers sont **at-least-once** (l'offset est validé après le handler). Un
message qui échoue 3 fois, ou qui n'est pas décodable, est écrit sur `<topic>.dlq`. Écrivez des handlers idempotents.

## En ligne de commande

Le binaire `sb` utilise exactement le même code que l'application (pratique en CI) :

```sh
cargo run -p sb-app --bin sb -- sample > projet.sb.json   # projet d'exemple
cargo run -p sb-app --bin sb -- validate projet.sb.json   # code de sortie 1 s'il y a des erreurs
cargo run -p sb-app --bin sb -- generate projet.sb.json ./out
```

## Tests

```sh
cargo test --workspace                 # modèle, validation, générateur, logique applicative
pnpm --dir ui test                     # logique TypeScript (typage, renommages en cascade)
cargo build -p sb-app --bin sb && pnpm --dir ui e2e   # scénarios Playwright, avec le vrai code Rust
```

Le test le plus important n'est pas dans cette liste : le workspace **généré** doit compiler
(`cargo run -p sb-codegen --example dump -- /tmp/gen && cd /tmp/gen && cargo check --workspace`).
La CI le fait.

## Limites actuelles

À savoir avant de s'en servir sérieusement :

- **Le code généré n'a pas été exécuté contre un broker Kafka** : il compile (sans warning) mais le
  comportement à l'exécution (consumers, DLQ, appels gRPC entre services) n'a pas été éprouvé de bout en bout.
- Les flèches du canvas sont **dérivées** du modèle : on ne peut pas les tracer à la souris, on configure
  les étapes dans l'inspecteur.
- Pas d'annuler/rétablir. L'export **écrase** les fichiers générés : la logique sur mesure passe par
  l'étape *Code Rust*, pas par l'édition des fichiers exportés.
- Renommer un champ met à jour les mappings qui le ciblent, mais pas les expressions qui le *lisent*
  (`input.ancien_nom`) : la validation les signale.
- Les événements sont du protobuf brut, sans schema registry.
- L'interface est en français uniquement.
- Les dialogues de fichiers natifs (GTK) n'ont pas été exercés automatiquement (les tests simulent les dialogues) ;
  `pnpm tauri build` (paquets `.deb` / AppImage) n'a pas été essayé. L'application en mode `tauri dev`
  a été lancée et pilotée dans un serveur X virtuel.

La conception et la feuille de route sont dans [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
