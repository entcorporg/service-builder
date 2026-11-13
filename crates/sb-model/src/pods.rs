//! How the project runs in containers, from the Pods page: the resources each container gets, the admin interfaces
//! that come with the stack, and the reverse proxy (Traefik) in front of it.
//!
//! Nothing here changes the generated Rust: it only shapes the compose file and the Traefik configuration that the
//! Pods page writes next to the workspace before it starts the stack.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The values `restart` accepts (the restart policies of compose).
pub const RESTART_POLICIES: [&str; 4] = ["no", "always", "on-failure", "unless-stopped"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct PodsConfig {
    /// The CPUs the containers may run on (`0-5`, `0,2,4`). Empty: any.
    pub cpuset: String,
    /// The restart policy of every container: `no`, `always`, `on-failure` or `unless-stopped`.
    pub restart: String,
    /// Memory of each generated service (`160m`, `1g`). Swap is capped to the same amount.
    pub service_memory: String,
    /// Memory of the Kafka broker.
    pub kafka_memory: String,
    /// Memory of PostgreSQL.
    pub postgres_memory: String,
    /// Publishes the ports of the services, and of the admin interfaces when Traefik is off, on `127.0.0.1`.
    pub publish_ports: bool,
    /// A file of `NAME=value` lines given to every generated service (secrets, keys, addresses): it stays out of the
    /// project. Empty: none.
    pub env_file: String,
    /// Adminer, a web interface for PostgreSQL.
    pub adminer: bool,
    /// Kafka UI (kafbat), a web interface for the broker.
    pub kafka_ui: bool,
    /// Garage, an S3-compatible store, for the services that have an object step: its buckets and the access key of
    /// the services are made by the stack. Off: the store is the one the environment file names.
    pub garage: bool,
    pub traefik: TraefikConfig,
}

impl Default for PodsConfig {
    fn default() -> Self {
        Self {
            cpuset: String::new(),
            restart: "unless-stopped".into(),
            service_memory: "160m".into(),
            kafka_memory: "768m".into(),
            postgres_memory: "256m".into(),
            publish_ports: true,
            env_file: String::new(),
            adminer: true,
            kafka_ui: true,
            garage: true,
            traefik: TraefikConfig::default(),
        }
    }
}

impl PodsConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The bucket the stack makes for the variable of an object step: `AVATAR_BUCKET` gives `avatar-bucket`. A bucket
/// name has 3 to 63 lower case letters, digits and `-`.
pub fn bucket_name(env: &str) -> String {
    let mut name: String = env
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    name = name.trim_matches('-').to_string();
    if name.len() < 3 {
        name = format!("bucket-{name}");
    }
    name.truncate(63);
    name.trim_end_matches('-').to_string()
}

/// The reverse proxy: one host per admin interface and per frontend, and the API of the services on its own host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TraefikConfig {
    pub enabled: bool,
    /// The public domain (`example.com`): HTTPS with Let's Encrypt certificates. Empty: plain HTTP on `*.localhost`.
    pub domain: String,
    /// The e-mail of the Let's Encrypt account. Required with a domain.
    pub acme_email: String,
    /// Port of the host for HTTP (80: Let's Encrypt needs it reachable from the internet).
    pub http_port: u16,
    /// Port of the host for HTTPS. Unused without a domain.
    pub https_port: u16,
    /// Serves the dashboard of Traefik on its own host.
    pub dashboard: bool,
    /// Protects the dashboard and the admin interfaces with basic authentication: one `user:hash` line as written by
    /// `htpasswd -nB user`. Empty: no authentication.
    pub admin_auth: String,
    /// Requests per second one client may send to the API, on average.
    pub rate_average: u32,
    /// Requests one client may send at once above the average.
    pub rate_burst: u32,
    pub hosts: TraefikHosts,
    /// The frontends served through the proxy: dev servers running on the host.
    pub frontends: Vec<Frontend>,
}

impl Default for TraefikConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            domain: String::new(),
            acme_email: String::new(),
            http_port: 80,
            https_port: 443,
            dashboard: true,
            admin_auth: String::new(),
            rate_average: 100,
            rate_burst: 200,
            hosts: TraefikHosts::default(),
            frontends: vec![],
        }
    }
}

impl TraefikConfig {
    /// HTTPS, with certificates: only with a public domain.
    pub fn tls(&self) -> bool {
        !self.domain.trim().is_empty()
    }

    /// `sub.domain`, or the domain itself for an empty `sub`. Without a domain: `sub.localhost`.
    pub fn fqdn(&self, sub: &str) -> String {
        let domain = match self.domain.trim() {
            "" => "localhost",
            d => d,
        };
        match sub.trim() {
            "" => domain.to_string(),
            sub => format!("{sub}.{domain}"),
        }
    }

    /// The address a browser reaches `sub` at, with the port when it is not the default of the scheme.
    pub fn url(&self, sub: &str) -> String {
        let host = self.fqdn(sub);
        match (self.tls(), self.http_port, self.https_port) {
            (true, _, 443) => format!("https://{host}"),
            (true, _, port) => format!("https://{host}:{port}"),
            (false, 80, _) => format!("http://{host}"),
            (false, port, _) => format!("http://{host}:{port}"),
        }
    }
}

/// The sub-domain of each host the proxy serves (`adminer` gives `adminer.example.com`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TraefikHosts {
    /// The HTTP routes of the services.
    pub api: String,
    pub dashboard: String,
    pub adminer: String,
    pub kafka_ui: String,
    /// Mailpit, when the project has a mail service.
    pub mailpit: String,
    /// Garage, when a service has an object step: where a browser sends and reads the files (signed URLs).
    pub s3: String,
}

impl Default for TraefikHosts {
    fn default() -> Self {
        Self {
            api: "api".into(),
            dashboard: "traefik".into(),
            adminer: "adminer".into(),
            kafka_ui: "kafka".into(),
            mailpit: "mail".into(),
            s3: "s3".into(),
        }
    }
}

/// A frontend behind the proxy: a dev server on the host (Vite, for instance, listening on every interface).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Frontend {
    /// Lower case letters, digits and `-`: names the routers and middlewares. For the services that read the
    /// `X-Frontend-ID` header, it is also the value of that header and the prefix (`/<name>`) of the calls.
    pub name: String,
    /// Its sub-domain. Empty: the domain itself.
    #[serde(default)]
    pub host: String,
    /// The port its dev server listens on, on the host.
    pub port: u16,
}

/// A memory amount of compose: a number and an optional unit (`b`, `k`, `m`, `g`).
pub fn is_memory(text: &str) -> bool {
    let digits = text.trim_end_matches(['b', 'k', 'm', 'g']);
    text.len() - digits.len() <= 1
        && !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|b| b.is_ascii_digit())
}

/// A list of CPUs: numbers and ranges separated by commas (`0-5`, `0,2,4-7`).
pub fn is_cpuset(text: &str) -> bool {
    let number = |s: &str| !s.is_empty() && s.len() <= 4 && s.bytes().all(|b| b.is_ascii_digit());
    text.split(',').all(|part| match part.split_once('-') {
        Some((from, to)) => {
            number(from) && number(to) && from.parse::<u32>().ok() <= to.parse::<u32>().ok()
        }
        None => number(part),
    })
}

/// One label of a host name: letters, digits and `-`, not at the ends.
fn is_label(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 63
        && !text.starts_with('-')
        && !text.ends_with('-')
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A host name in lower case: labels separated by dots (`example.com`, `adminer`, `a.b`).
pub fn is_host(text: &str) -> bool {
    text.len() <= 253 && text.split('.').all(is_label)
}

/// A name usable in a router or a middleware: a single label starting with a letter.
pub fn is_frontend_name(text: &str) -> bool {
    is_label(text) && text.starts_with(|c: char| c.is_ascii_lowercase())
}

/// A `user:hash` line of htpasswd, on one line and without what would break out of a YAML string.
pub fn is_htpasswd(text: &str) -> bool {
    text.split_once(':').is_some_and(|(user, hash)| {
        !user.is_empty()
            && !hash.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_cpus_and_hosts_are_checked() {
        for ok in ["160m", "1g", "64", "512k"] {
            assert!(is_memory(ok), "{ok}");
        }
        for bad in ["", "m", "0m", "1.5g", "160M", "160mb", "-1m"] {
            assert!(!is_memory(bad), "{bad}");
        }
        for ok in ["0", "0-5", "0,2,4-7"] {
            assert!(is_cpuset(ok), "{ok}");
        }
        for bad in ["", "a", "5-0", "0-", "0,,1", "0 - 5"] {
            assert!(!is_cpuset(bad), "{bad}");
        }
        assert!(is_host("example.com") && is_host("adminer") && is_host("a-b.c"));
        assert!(!is_host("") && !is_host("Example.com") && !is_host("-a.com") && !is_host("a..b"));
        assert!(is_htpasswd("admin:$2y$05$abc.def/ghi"));
        assert!(!is_htpasswd("admin") && !is_htpasswd("a:b\nc:d") && !is_htpasswd("a:\"b"));
    }

    #[test]
    fn a_bucket_is_named_after_its_variable() {
        assert_eq!(bucket_name("AVATAR_BUCKET"), "avatar-bucket");
        assert_eq!(bucket_name("_B_"), "bucket-b");
        let long = bucket_name(&format!("{}_X", "A".repeat(62)));
        assert_eq!(long, "a".repeat(62));
    }

    #[test]
    fn hosts_follow_the_domain() {
        let mut t = TraefikConfig::default();
        assert_eq!(t.url("adminer"), "http://adminer.localhost");
        assert_eq!(t.url(""), "http://localhost");
        t.http_port = 8080;
        assert_eq!(t.url("api"), "http://api.localhost:8080");
        t.domain = "example.com".into();
        assert_eq!(t.url("api"), "https://api.example.com");
        assert_eq!(t.url(""), "https://example.com");
        t.https_port = 8443;
        assert_eq!(t.url("api"), "https://api.example.com:8443");
    }

    #[test]
    fn a_default_configuration_is_not_written() {
        let project = crate::Project::new("demo");
        let json = serde_json::to_value(&project).unwrap();
        assert!(json.get("pods").is_none());
        // And a file that sets one value keeps the defaults of the others.
        let pods: PodsConfig = serde_json::from_str(r#"{ "cpuset": "0-5" }"#).unwrap();
        assert_eq!(pods.service_memory, "160m");
        assert!(pods.adminer && !pods.traefik.enabled);
    }

    fn codes(project: &crate::Project) -> Vec<String> {
        crate::validate(project)
            .into_iter()
            .filter(|d| d.path.starts_with("/pods"))
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn the_settings_are_validated() {
        let mut p = crate::sample::orders();
        assert!(codes(&p).is_empty());
        p.pods.cpuset = "all".into();
        p.pods.restart = "sometimes".into();
        p.pods.service_memory = "lots".into();
        assert_eq!(
            codes(&p),
            ["E_PODS_CPUSET", "E_PODS_RESTART", "E_PODS_MEMORY"]
        );

        // The proxy is only checked when it is on.
        let mut p = crate::sample::orders();
        p.pods.traefik.domain = "Not A Domain".into();
        assert!(codes(&p).is_empty());
        p.pods.traefik.enabled = true;
        assert_eq!(
            codes(&p),
            ["E_PODS_DOMAIN", "E_PODS_ACME_EMAIL", "W_PODS_ADMIN_OPEN"]
        );

        // A public domain without authentication is only a warning.
        p.pods.traefik.domain = "example.com".into();
        p.pods.traefik.acme_email = "ops@example.com".into();
        assert_eq!(codes(&p), ["W_PODS_ADMIN_OPEN"]);
        assert!(!crate::has_errors(&crate::validate(&p)));
        p.pods.traefik.admin_auth = "admin:$2y$05$hash".into();
        assert!(codes(&p).is_empty());

        // The host of the object store counts only when the stack has one.
        p.pods.traefik.hosts.s3 = "adminer".into();
        assert!(codes(&p).is_empty());
        crate::sample::add_jwt(&mut p);
        crate::sample::add_avatars(&mut p);
        assert_eq!(codes(&p), ["E_PODS_HOST_DUPLICATE"]);
        p.pods.garage = false;
        assert!(codes(&p).is_empty());
        let mut p = {
            let mut fresh = crate::sample::orders();
            fresh.pods = p.pods.clone();
            fresh.pods.garage = true;
            fresh.pods.traefik.hosts.s3 = "s3".into();
            fresh
        };

        // Frontends: names and hosts are each unique.
        let front = |name: &str, host: &str| Frontend {
            name: name.into(),
            host: host.into(),
            port: 5173,
        };
        p.pods.traefik.frontends = vec![
            front("admin", "admin"),
            front("admin", "adminer"),
            front("Site", ""),
        ];
        assert_eq!(
            codes(&p),
            [
                "E_PODS_FRONTEND_NAME",
                "E_PODS_FRONTEND_NAME",
                "E_PODS_HOST_DUPLICATE",
            ]
        );
    }
}
