//! What agents using the MCP server think Service Builder is missing.
//!
//! Every [`REPORT_EVERY`] tool calls the server asks the agent for a short report; the agent files
//! it with `submit_report`. Reports wait next to the project until the user reads them and decides,
//! one by one, to open a GitHub issue (a pre-filled page in their browser): nothing leaves the
//! machine on its own, and no token is needed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A report is requested after this many tool calls.
pub const REPORT_EVERY: u32 = 10;
/// Reports waiting for the user; beyond that agents are told to stop.
pub const MAX_PENDING: usize = 20;
/// Where the issues go.
pub const REPOSITORY: &str = "entcorporg/service-builder";

const MAX_SUGGESTIONS: usize = 5;
const MAX_SUMMARY: usize = 500;
const MAX_TITLE: usize = 120;
const MAX_DETAILS: usize = 1500;
/// GitHub refuses very long URLs; the body is cut to stay under this.
const MAX_URL: usize = 7000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub title: String,
    #[serde(default)]
    pub details: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportStatus {
    /// Not looked at yet.
    Pending,
    /// The user opened the GitHub page for it.
    Opened,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: u32,
    /// Seconds since the Unix epoch.
    pub created_at: u64,
    #[serde(default)]
    pub summary: String,
    pub suggestions: Vec<Suggestion>,
    pub status: ReportStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feedback {
    /// Whether agents are asked for reports at all.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Tool calls since the last request.
    #[serde(default)]
    pub interactions: u32,
    #[serde(default)]
    pub next_id: u32,
    #[serde(default)]
    pub reports: Vec<Report>,
}

fn yes() -> bool {
    true
}

impl Default for Feedback {
    fn default() -> Self {
        Feedback {
            enabled: true,
            interactions: 0,
            next_id: 1,
            reports: vec![],
        }
    }
}

/// Kept in a hidden folder next to the project file, apart from the project itself.
pub fn state_path(project_file: &Path) -> PathBuf {
    let dir = project_file.parent().unwrap_or(Path::new("."));
    dir.join(".service-builder").join(format!(
        "{}.mcp.json",
        crate::mcp::project_slug(project_file)
    ))
}

pub fn load(project_file: &Path) -> Result<Feedback, String> {
    let path = state_path(project_file);
    match fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| format!("{} is not valid: {e}", path.display()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Feedback::default()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn save(project_file: &Path, feedback: &Feedback) -> Result<(), String> {
    let path = state_path(project_file);
    let dir = path.parent().expect("state path has a parent");
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(feedback).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json + "\n").map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Counts one tool call; true when it is time to ask the agent for a report. A broken state
/// file never gets in the way of the tool call itself.
pub fn tick(project_file: &Path) -> bool {
    let Ok(mut feedback) = load(project_file) else {
        return false;
    };
    if !feedback.enabled {
        return false;
    }
    feedback.interactions += 1;
    let due = feedback.interactions >= REPORT_EVERY;
    if due {
        feedback.interactions = 0;
    }
    if let Err(e) = save(project_file, &feedback) {
        eprintln!("cannot save the feedback counter: {e}");
    }
    due && feedback
        .reports
        .iter()
        .filter(|r| r.status == ReportStatus::Pending)
        .count()
        < MAX_PENDING
}

/// What the agent reads when a report is due.
pub fn request_text() -> String {
    format!(
        "Service Builder feedback request (once every {REPORT_EVERY} calls). If, while working on this \
         project, a block, field, tool or check was missing that would have made your work easier or \
         safer (for example a logic block that runs blocking work such as password hashing in \
         `spawn_blocking`), call `submit_report` now with 1 to {MAX_SUGGESTIONS} concrete suggestions. \
         Describe the general need only: no project names, secrets or file contents, because the user \
         reviews the report and may publish it as a public GitHub issue. If you have nothing to \
         suggest, ignore this message."
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn text_arg(value: &Value, key: &str, max: usize) -> Result<String, String> {
    let text = value.get(key).and_then(Value::as_str).unwrap_or("").trim();
    if text.chars().count() > max {
        return Err(format!("`{key}` is longer than {max} characters"));
    }
    Ok(text.to_string())
}

/// Files the agent's report. Returns the acknowledgement the agent reads.
pub fn submit(project_file: &Path, args: &Value) -> Result<String, String> {
    let summary = text_arg(args, "summary", MAX_SUMMARY)?;
    let raw = args
        .get("suggestions")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty())
        .ok_or("`suggestions` must be a non-empty array")?;
    if raw.len() > MAX_SUGGESTIONS {
        return Err(format!("at most {MAX_SUGGESTIONS} suggestions per report"));
    }
    let mut suggestions = Vec::new();
    for (i, s) in raw.iter().enumerate() {
        let title = text_arg(s, "title", MAX_TITLE).map_err(|e| format!("suggestion {i}: {e}"))?;
        if title.is_empty() {
            return Err(format!("suggestion {i}: `title` is required"));
        }
        let details =
            text_arg(s, "details", MAX_DETAILS).map_err(|e| format!("suggestion {i}: {e}"))?;
        suggestions.push(Suggestion { title, details });
    }

    let mut feedback = load(project_file)?;
    if feedback
        .reports
        .iter()
        .filter(|r| r.status == ReportStatus::Pending)
        .count()
        >= MAX_PENDING
    {
        return Err(
            "too many reports are waiting for the user to read them; do not send more".into(),
        );
    }
    let id = feedback
        .next_id
        .max(feedback.reports.iter().map(|r| r.id + 1).max().unwrap_or(1));
    feedback.next_id = id + 1;
    feedback.reports.push(Report {
        id,
        created_at: now(),
        summary,
        suggestions,
        status: ReportStatus::Pending,
    });
    save(project_file, &feedback)?;
    Ok("Thank you: the report was saved. The user decides whether it is sent to GitHub.".into())
}

pub fn set_enabled(project_file: &Path, enabled: bool) -> Result<Feedback, String> {
    let mut feedback = load(project_file)?;
    feedback.enabled = enabled;
    feedback.interactions = 0;
    save(project_file, &feedback)?;
    Ok(feedback)
}

pub fn dismiss(project_file: &Path, id: u32) -> Result<Feedback, String> {
    let mut feedback = load(project_file)?;
    feedback.reports.retain(|r| r.id != id);
    save(project_file, &feedback)?;
    Ok(feedback)
}

/// The pre-filled "new issue" page for a report; the report is marked as opened.
pub fn issue_url_for(project_file: &Path, id: u32) -> Result<String, String> {
    let mut feedback = load(project_file)?;
    let report = feedback
        .reports
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("no report {id}"))?;
    report.status = ReportStatus::Opened;
    let url = issue_url(report);
    save(project_file, &feedback)?;
    Ok(url)
}

fn issue_title(report: &Report) -> String {
    let first = &report.suggestions[0].title;
    match report.suggestions.len() {
        1 => format!("[Suggestion d’agent] {first}"),
        n => format!("[Suggestion d’agent] {first} (+{})", n - 1),
    }
}

fn issue_body(report: &Report) -> String {
    let mut body = String::new();
    if !report.summary.is_empty() {
        body.push_str(&format!("## Résumé\n\n{}\n\n", report.summary));
    }
    body.push_str("## Suggestions\n\n");
    for (i, s) in report.suggestions.iter().enumerate() {
        body.push_str(&format!("### {}. {}\n\n", i + 1, s.title));
        if !s.details.is_empty() {
            body.push_str(&format!("{}\n\n", s.details));
        }
    }
    body.push_str(&format!(
        "---\nRapport d’un agent via le MCP de Service Builder v{}, relu par l’utilisateur avant publication.\n",
        env!("CARGO_PKG_VERSION")
    ));
    body
}

/// Percent-encodes everything but the RFC 3986 unreserved characters.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn issue_url(report: &Report) -> String {
    const CUT: &str = "\n\n(texte tronqué)";
    let base = format!(
        "https://github.com/{REPOSITORY}/issues/new?labels=agent-suggestion&title={}&body=",
        encode(&issue_title(report))
    );
    let chars: Vec<char> = issue_body(report).chars().collect();
    let url_for = |keep: usize| {
        let mut body: String = chars[..keep].iter().collect();
        if keep < chars.len() {
            body.push_str(CUT);
        }
        format!("{base}{}", encode(&body))
    };
    // Keep as much of the body as fits, shrinking by a fifth each round.
    let mut keep = chars.len();
    loop {
        let url = url_for(keep);
        if url.len() <= MAX_URL || keep == 0 {
            return url;
        }
        keep = keep * 4 / 5;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sb-feedback-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("demo.sb.json");
        (dir, file)
    }

    fn report_args() -> Value {
        json!({
            "summary": "Hashing a password blocks the async runtime.",
            "suggestions": [{ "title": "Blocking task block", "details": "A step that runs code in tokio::task::spawn_blocking." }]
        })
    }

    #[test]
    fn asks_every_fifty_calls_then_starts_over() {
        let (dir, file) = project("tick");
        for call in 1..=REPORT_EVERY * 2 {
            let due = tick(&file);
            assert_eq!(due, call % REPORT_EVERY == 0, "call {call}");
        }
        assert_eq!(load(&file).unwrap().interactions, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn never_asks_when_disabled_or_when_too_many_reports_wait() {
        let (dir, file) = project("off");
        set_enabled(&file, false).unwrap();
        assert!((0..REPORT_EVERY * 2).all(|_| !tick(&file)));
        set_enabled(&file, true).unwrap();
        for _ in 0..MAX_PENDING {
            submit(&file, &report_args()).unwrap();
        }
        assert!((0..REPORT_EVERY).all(|_| !tick(&file)));
        assert!(
            submit(&file, &report_args())
                .unwrap_err()
                .contains("too many")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stores_reports_next_to_the_project_and_numbers_them() {
        let (dir, file) = project("store");
        submit(&file, &report_args()).unwrap();
        submit(&file, &report_args()).unwrap();
        let feedback = load(&file).unwrap();
        assert_eq!(
            feedback.reports.iter().map(|r| r.id).collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(state_path(&file).starts_with(dir.join(".service-builder")));
        assert!(feedback.reports[0].created_at > 0);
        // Dismissing never reuses a number.
        dismiss(&file, 2).unwrap();
        submit(&file, &report_args()).unwrap();
        assert_eq!(
            load(&file)
                .unwrap()
                .reports
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_malformed_reports_without_storing_them() {
        let (dir, file) = project("bad");
        let bad = [
            json!({}),
            json!({ "suggestions": [] }),
            json!({ "suggestions": [{ "details": "no title" }] }),
            json!({ "suggestions": [{ "title": "x".repeat(MAX_TITLE + 1) }] }),
            json!({ "summary": "x".repeat(MAX_SUMMARY + 1), "suggestions": [{ "title": "t" }] }),
            json!({ "suggestions": (0..=MAX_SUGGESTIONS).map(|i| json!({ "title": format!("t{i}") })).collect::<Vec<_>>() }),
        ];
        for args in bad {
            assert!(submit(&file, &args).is_err(), "{args}");
        }
        assert!(!state_path(&file).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_broken_state_file_does_not_block_tool_calls() {
        let (dir, file) = project("broken");
        fs::create_dir_all(state_path(&file).parent().unwrap()).unwrap();
        fs::write(state_path(&file), "{ nope").unwrap();
        assert!(!tick(&file));
        assert_eq!(fs::read_to_string(state_path(&file)).unwrap(), "{ nope");
        assert!(load(&file).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn builds_a_github_issue_url_and_marks_the_report_opened() {
        let (dir, file) = project("url");
        submit(&file, &report_args()).unwrap();
        let url = issue_url_for(&file, 1).unwrap();
        assert!(url.starts_with("https://github.com/entcorporg/service-builder/issues/new?labels=agent-suggestion&title="));
        assert!(
            url.contains("title=%5BSuggestion%20d%E2%80%99agent%5D%20Blocking%20task%20block&")
        );
        assert!(url.contains("spawn_blocking"));
        assert!(!url.contains(' ') && !url.contains('\n'));
        assert_eq!(load(&file).unwrap().reports[0].status, ReportStatus::Opened);
        assert!(issue_url_for(&file, 9).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn long_reports_are_cut_to_fit_the_url_and_say_so() {
        let report = Report {
            id: 1,
            created_at: 0,
            summary: String::new(),
            suggestions: (0..5)
                .map(|i| Suggestion {
                    title: format!("Idée {i}"),
                    details: "é".repeat(MAX_DETAILS),
                })
                .collect(),
            status: ReportStatus::Pending,
        };
        let url = issue_url(&report);
        assert!(url.len() <= MAX_URL, "{} bytes", url.len());
        assert!(
            url.ends_with(&encode("(texte tronqué)")),
            "{}",
            &url[url.len() - 60..]
        );
        assert!(issue_title(&report).ends_with("(+4)"));
    }
}
