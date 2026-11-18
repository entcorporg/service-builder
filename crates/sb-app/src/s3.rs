//! A small S3 client: just what the project store needs (put, get, list), against Garage or any S3-compatible
//! server, path-style (`<endpoint>/<bucket>/<key>`), requests signed with AWS Signature Version 4.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where the objects are, and the key to reach them with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct S3Config {
    /// `http://127.0.0.1:3900` for a local Garage.
    pub endpoint: String,
    /// Garage answers to any region name, `garage` by default.
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:3900".into(),
            region: "garage".into(),
            bucket: "service-builder".into(),
            access_key: String::new(),
            secret_key: String::new(),
        }
    }
}

impl S3Config {
    /// What is needed to talk to the server: an access key is the only thing that has no default.
    pub fn is_complete(&self) -> bool {
        [
            &self.endpoint,
            &self.region,
            &self.bucket,
            &self.access_key,
            &self.secret_key,
        ]
        .iter()
        .all(|v| !v.trim().is_empty())
    }
}

/// One object of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub key: String,
    /// `2026-11-18T12:46:03.000Z`, as the server writes it.
    pub modified: String,
    pub size: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Listing {
    pub objects: Vec<Object>,
    /// The "folders" met when a delimiter was given: `prefix/`.
    pub prefixes: Vec<String>,
}

#[derive(Debug)]
pub struct Client {
    config: S3Config,
    agent: ureq::Agent,
}

type HmacSha256 = Hmac<Sha256>;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Everything but the unreserved characters is percent-encoded; `/` too unless `keep_slash` (a path).
fn encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Days since 1970-01-01 to a civil date (proleptic Gregorian), after Howard Hinnant's algorithm.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `(year, month, day, hour, minute, second)` of a Unix time, in UTC.
fn utc(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let (y, m, d) = civil((secs / 86_400) as i64);
    let rest = secs % 86_400;
    (
        y,
        m,
        d,
        (rest / 3600) as u32,
        (rest % 3600 / 60) as u32,
        (rest % 60) as u32,
    )
}

/// `20260118T124603Z`, the date of a signed request.
pub fn amz_date(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = utc(secs);
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z")
}

/// `2026-01-18T12:46:03Z`.
pub fn iso8601(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = utc(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The `Authorization` header of a request. `headers` are the ones that are signed (names in lower case, sorted),
/// `path` the already encoded path, `query` the already encoded `name=value` pairs.
#[allow(clippy::too_many_arguments)]
fn authorization(
    config: &S3Config,
    method: &str,
    path: &str,
    query: &[(String, String)],
    headers: &[(String, String)],
    payload_hash: &str,
    amz: &str,
) -> String {
    let mut query: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
    query.sort();
    let canonical_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let signed: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    let signed = signed.join(";");
    let canonical = format!(
        "{method}\n{path}\n{}\n{canonical_headers}\n{signed}\n{payload_hash}",
        query.join("&")
    );
    let date = &amz[..8];
    let scope = format!("{date}/{}/s3/aws4_request", config.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz}\n{scope}\n{}",
        sha256_hex(canonical.as_bytes())
    );
    let k = hmac(format!("AWS4{}", config.secret_key).as_bytes(), date);
    let k = hmac(&k, &config.region);
    let k = hmac(&k, "s3");
    let k = hmac(&k, "aws4_request");
    let signature = hex(&hmac(&k, &to_sign));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
        config.access_key
    )
}

impl Client {
    pub fn new(config: &S3Config) -> Result<Self, String> {
        if !config.is_complete() {
            return Err("le stockage interne n'est pas prêt".into());
        }
        let endpoint = config.endpoint.trim().trim_end_matches('/');
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Err(format!(
                "{endpoint} : l'adresse doit commencer par http:// ou https://"
            ));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent(format!("service-builder/{}", env!("CARGO_PKG_VERSION")))
            .timeout_global(Some(Duration::from_secs(20)))
            // A 404 is an answer (no such object), not a failure.
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self {
            config: S3Config {
                endpoint: endpoint.to_string(),
                ..config.clone()
            },
            agent,
        })
    }

    fn host(&self) -> &str {
        let rest = self
            .config
            .endpoint
            .split_once("://")
            .map_or(self.config.endpoint.as_str(), |(_, r)| r);
        rest.split('/').next().unwrap_or(rest)
    }

    /// Sends one signed request. `key` empty: the bucket itself.
    fn send(
        &self,
        method: &str,
        key: &str,
        query: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<(u16, Vec<u8>), String> {
        let path = if key.is_empty() {
            format!("/{}", encode(&self.config.bucket, false))
        } else {
            format!(
                "/{}/{}",
                encode(&self.config.bucket, false),
                encode(key, true)
            )
        };
        let query: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| (encode(k, false), encode(v, false)))
            .collect();
        let payload_hash = sha256_hex(body.unwrap_or_default());
        let amz = amz_date(now());
        let headers = vec![
            ("host".to_string(), self.host().to_string()),
            ("x-amz-content-sha256".to_string(), payload_hash.clone()),
            ("x-amz-date".to_string(), amz.clone()),
        ];
        let auth = authorization(
            &self.config,
            method,
            &path,
            &query,
            &headers,
            &payload_hash,
            &amz,
        );
        let mut url = format!("{}{path}", self.config.endpoint);
        if !query.is_empty() {
            let pairs: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
            url.push('?');
            url.push_str(&pairs.join("&"));
        }
        let fail = |e: ureq::Error| format!("{} : {e}", self.config.endpoint);
        let mut response = match method {
            "PUT" => self
                .agent
                .put(&url)
                .header("x-amz-content-sha256", &payload_hash)
                .header("x-amz-date", &amz)
                .header("authorization", &auth)
                .send(body.unwrap_or_default()),
            _ => self
                .agent
                .get(&url)
                .header("x-amz-content-sha256", &payload_hash)
                .header("x-amz-date", &amz)
                .header("authorization", &auth)
                .call(),
        }
        .map_err(fail)?;
        let status = response.status().as_u16();
        let bytes = response.body_mut().read_to_vec().map_err(fail)?;
        Ok((status, bytes))
    }

    fn refused(&self, what: &str, status: u16, body: &[u8]) -> String {
        let text = String::from_utf8_lossy(body);
        let detail =
            tag(&text, "Message").unwrap_or_else(|| text.trim().chars().take(160).collect());
        format!("{what} : le serveur répond {status} ({detail})")
    }

    pub fn put(&self, key: &str, data: &[u8]) -> Result<(), String> {
        let (status, body) = self.send("PUT", key, &[], Some(data))?;
        match status {
            200..=299 => Ok(()),
            _ => Err(self.refused(&format!("écriture de {key}"), status, &body)),
        }
    }

    /// `None` when there is no such object.
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let (status, body) = self.send("GET", key, &[], None)?;
        match status {
            200..=299 => Ok(Some(body)),
            404 => Ok(None),
            _ => Err(self.refused(&format!("lecture de {key}"), status, &body)),
        }
    }

    /// Every object under `prefix` (all the pages), and with a `delimiter` the folders directly under it.
    pub fn list(&self, prefix: &str, delimiter: Option<&str>) -> Result<Listing, String> {
        let mut all = Listing::default();
        let mut token = String::new();
        loop {
            let mut query = vec![("list-type", "2"), ("prefix", prefix)];
            if let Some(d) = delimiter {
                query.push(("delimiter", d));
            }
            if !token.is_empty() {
                query.push(("continuation-token", &token));
            }
            let (status, body) = self.send("GET", "", &query, None)?;
            if !(200..300).contains(&status) {
                return Err(self.refused("liste du bucket", status, &body));
            }
            let page = parse_listing(&String::from_utf8_lossy(&body));
            all.objects.extend(page.0.objects);
            all.prefixes.extend(page.0.prefixes);
            match page.1 {
                Some(next) => token = next,
                None => return Ok(all),
            }
        }
    }

    /// Reaches the server and the bucket with this key.
    pub fn check(&self) -> Result<(), String> {
        let (status, body) =
            self.send("GET", "", &[("list-type", "2"), ("max-keys", "1")], None)?;
        match status {
            200..=299 => Ok(()),
            404 => Err(format!("le bucket « {} » n'existe pas", self.config.bucket)),
            _ => Err(self.refused("connexion", status, &body)),
        }
    }
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text of the first `<name>…</name>` of `xml`.
fn tag(xml: &str, name: &str) -> Option<String> {
    let start = xml.find(&format!("<{name}>"))? + name.len() + 2;
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(unescape(&xml[start..end]))
}

/// The pieces of `xml` between `<name>` and `</name>`.
fn blocks<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let mut out = vec![];
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(&after[..j]);
        rest = &after[j + close.len()..];
    }
    out
}

/// One page of a `ListObjectsV2` answer, and the token of the next one when it is truncated.
fn parse_listing(xml: &str) -> (Listing, Option<String>) {
    let objects = blocks(xml, "Contents")
        .into_iter()
        .filter_map(|c| {
            Some(Object {
                key: tag(c, "Key")?,
                modified: tag(c, "LastModified").unwrap_or_default(),
                size: tag(c, "Size").and_then(|s| s.parse().ok()).unwrap_or(0),
            })
        })
        .collect();
    let prefixes = blocks(xml, "CommonPrefixes")
        .into_iter()
        .filter_map(|c| tag(c, "Prefix"))
        .collect();
    let next = (tag(xml, "IsTruncated").as_deref() == Some("true"))
        .then(|| tag(xml, "NextContinuationToken"))
        .flatten();
    (Listing { objects, prefixes }, next)
}

/// A server in memory that speaks enough S3 (put, get, list) for the tests of this crate.
#[cfg(test)]
pub(crate) mod testing {
    use super::S3Config;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// What the server holds, by key.
    pub(crate) type Memory = Arc<Mutex<std::collections::BTreeMap<String, Vec<u8>>>>;

    /// A server that keeps what it is given, enough for put, get and list.
    pub(crate) fn fake_server() -> (S3Config, Memory) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let store: Memory = Default::default();
        let shared = store.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = vec![];
                let mut chunk = [0u8; 4096];
                // The head, then as many bytes of body as it announces.
                let (head, mut body) = loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break (String::new(), vec![]);
                    }
                    buffer.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        break (
                            String::from_utf8_lossy(&buffer[..i]).to_string(),
                            buffer[i + 4..].to_vec(),
                        );
                    }
                };
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while body.len() < length {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..n]);
                }
                let mut first = head.lines().next().unwrap_or("").split(' ');
                let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
                let authorized = head.contains("AWS4-HMAC-SHA256 Credential=test-key/");
                let (path, query) = target.split_once('?').unwrap_or((target, ""));
                let key = path
                    .trim_start_matches("/bucket")
                    .trim_start_matches('/')
                    .to_string();
                let reply = |status: &str, body: Vec<u8>| {
                    let mut out = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    out.extend(body);
                    out
                };
                let out = if !authorized {
                    reply(
                        "403 Forbidden",
                        b"<Error><Message>no key</Message></Error>".to_vec(),
                    )
                } else if method == "PUT" {
                    shared.lock().unwrap().insert(key, body);
                    reply("200 OK", vec![])
                } else if key.is_empty() {
                    let prefix = query
                        .split('&')
                        .find_map(|p| p.strip_prefix("prefix="))
                        .unwrap_or("")
                        .replace("%2F", "/");
                    let delimiter = query.contains("delimiter=");
                    let mut xml =
                        String::from("<ListBucketResult><IsTruncated>false</IsTruncated>");
                    let mut seen = std::collections::BTreeSet::new();
                    for (k, v) in shared.lock().unwrap().iter() {
                        let Some(rest) = k.strip_prefix(&prefix) else {
                            continue;
                        };
                        match rest.split_once('/') {
                            Some((folder, _)) if delimiter => {
                                if seen.insert(folder.to_string()) {
                                    xml.push_str(&format!("<CommonPrefixes><Prefix>{prefix}{folder}/</Prefix></CommonPrefixes>"));
                                }
                            }
                            _ => xml.push_str(&format!("<Contents><Key>{k}</Key><LastModified>2026-11-18T12:00:00.000Z</LastModified><Size>{}</Size></Contents>", v.len())),
                        }
                    }
                    xml.push_str("</ListBucketResult>");
                    reply("200 OK", xml.into_bytes())
                } else {
                    match shared.lock().unwrap().get(&key) {
                        Some(v) => reply("200 OK", v.clone()),
                        None => reply(
                            "404 Not Found",
                            b"<Error><Code>NoSuchKey</Code></Error>".to_vec(),
                        ),
                    }
                };
                let _ = stream.write_all(&out);
            }
        });
        let config = S3Config {
            endpoint: format!("http://127.0.0.1:{port}"),
            region: "garage".into(),
            bucket: "bucket".into(),
            access_key: "test-key".into(),
            secret_key: "secret".into(),
        };
        (config, store)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::fake_server;
    use super::*;

    #[test]
    fn the_signature_is_the_one_of_the_aws_documentation() {
        // "Example: GET Object" of the SigV4 documentation, with its documented signature.
        let config = S3Config {
            endpoint: "https://examplebucket.s3.amazonaws.com".into(),
            region: "us-east-1".into(),
            bucket: "examplebucket".into(),
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        };
        let empty = sha256_hex(b"");
        let headers = vec![
            (
                "host".to_string(),
                "examplebucket.s3.amazonaws.com".to_string(),
            ),
            ("range".to_string(), "bytes=0-9".to_string()),
            ("x-amz-content-sha256".to_string(), empty.clone()),
            ("x-amz-date".to_string(), "20130524T000000Z".to_string()),
        ];
        let auth = authorization(
            &config,
            "GET",
            "/test.txt",
            &[],
            &headers,
            &empty,
            "20130524T000000Z",
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn dates_are_utc() {
        assert_eq!(amz_date(1_369_353_600), "20130524T000000Z");
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_782_000_000), "2026-06-21T00:00:00Z");
        // A leap day.
        assert_eq!(iso8601(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn keys_and_queries_are_encoded_as_s3_wants() {
        assert_eq!(encode("a b/é+c", true), "a%20b/%C3%A9%2Bc");
        assert_eq!(encode("a/b", false), "a%2Fb");
        assert_eq!(encode("AZaz09-_.~", false), "AZaz09-_.~");
    }

    #[test]
    fn a_listing_is_read_page_after_page() {
        let xml = "<ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>tok&amp;1</NextContinuationToken>\
            <Contents><Key>a/versions/000001.json</Key><LastModified>2026-11-18T12:46:03.000Z</LastModified><Size>12</Size></Contents>\
            <Contents><Key>a/meta.json</Key><Size>3</Size></Contents>\
            <CommonPrefixes><Prefix>a/</Prefix></CommonPrefixes></ListBucketResult>";
        let (page, next) = parse_listing(xml);
        assert_eq!(next.as_deref(), Some("tok&1"));
        assert_eq!(page.objects.len(), 2);
        assert_eq!(page.objects[0].size, 12);
        assert_eq!(page.objects[0].modified, "2026-11-18T12:46:03.000Z");
        assert_eq!(page.prefixes, ["a/"]);
        assert_eq!(
            parse_listing("<ListBucketResult></ListBucketResult>").1,
            None
        );
    }

    #[test]
    fn objects_are_put_read_and_listed() {
        let (config, _) = fake_server();
        let client = Client::new(&config).unwrap();
        client.check().unwrap();
        client
            .put("orders/versions/000001.json", b"{\"a\":1}")
            .unwrap();
        client.put("orders/meta.json", b"{}").unwrap();
        client.put("clairtyx/meta.json", b"{}").unwrap();
        assert_eq!(client.get("orders/meta.json").unwrap().unwrap(), b"{}");
        assert_eq!(client.get("orders/nope.json").unwrap(), None);
        let folders = client.list("", Some("/")).unwrap().prefixes;
        assert_eq!(folders, ["clairtyx/", "orders/"]);
        let versions = client.list("orders/versions/", None).unwrap().objects;
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].size, 7);

        // A wrong key is refused with the message of the server.
        let wrong = Client::new(&S3Config {
            access_key: "other".into(),
            ..config
        })
        .unwrap();
        assert!(wrong.check().unwrap_err().contains("403"));
    }

    #[test]
    fn an_unreachable_server_or_a_missing_setting_is_an_error_not_a_hang() {
        assert!(
            Client::new(&S3Config::default())
                .unwrap_err()
                .contains("pas prêt")
        );
        let config = S3Config {
            endpoint: "http://127.0.0.1:9".into(),
            access_key: "k".into(),
            secret_key: "s".into(),
            ..Default::default()
        };
        assert!(Client::new(&config).unwrap().check().is_err());
        assert!(
            Client::new(&S3Config {
                endpoint: "ftp://x".into(),
                ..config
            })
            .is_err()
        );
    }
}
