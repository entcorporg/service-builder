//! A Model Context Protocol server over stdio, so that an AI client (Claude Code, Claude Desktop…)
//! can read, edit, validate and export a Service Builder project.
//!
//! The server is stateless: every call reads the project file and every edit writes it back, so
//! it always sees what the desktop app saved and the app sees what the client changed. Stdout
//! carries nothing but protocol messages (one JSON object per line); diagnostics go to stderr.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use sb_model::{Project, Severity, has_errors};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{analyze, export_project, load_project, save_project};

const PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const PARSE_ERROR: i64 = -32700;

/// A tool as the desktop app lists it.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSummary {
    pub name: &'static str,
    pub description: &'static str,
}

struct ToolDef {
    name: &'static str,
    description: &'static str,
    /// Properties of the input object and the ones that are required.
    properties: fn() -> Value,
    required: &'static [&'static str],
}

fn no_properties() -> Value {
    json!({})
}

const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "describe_project",
        description: "Outline of the project: messages, topics, and for each service its RPCs, routes, tables and consumers. Start here, then read details with get_project.",
        properties: no_properties,
        required: &[],
    },
    ToolDef {
        name: "get_project",
        description: "The whole project as JSON, exactly as stored in the project file.",
        properties: no_properties,
        required: &[],
    },
    ToolDef {
        name: "get_schema",
        description: "JSON Schema of the project file, derived from the model itself: every step type, field, enum value and operator. Without `definition` it lists the definitions; with one (for example Step, Expr, Condition, SqlCondition) it returns that definition with the ones it references.",
        properties: || json!({ "definition": { "type": "string", "description": "Name of a definition, as listed by get_schema without argument." } }),
        required: &[],
    },
    ToolDef {
        name: "get_example_project",
        description: "A complete example project (Orders, Payments, Notifications) to copy the JSON structure from. Does not touch the project file.",
        properties: no_properties,
        required: &[],
    },
    ToolDef {
        name: "patch_project",
        description: "Edits the project with RFC 6902 operations (add, replace, remove; JSON pointers like /services/0/routes/-), validates the result and saves it. Nothing is written if an operation fails or the result is not a project. Returns the diagnostics.",
        properties: || {
            json!({ "operations": {
                "type": "array",
                "description": "Applied in order.",
                "items": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["add", "replace", "remove"] },
                        "path": { "type": "string", "description": "JSON pointer, e.g. /topics/- or /services/0/name" },
                        "value": { "description": "New value for add and replace." }
                    },
                    "required": ["op", "path"]
                }
            }})
        },
        required: &["operations"],
    },
    ToolDef {
        name: "set_project",
        description: "Replaces the whole project with the given JSON, validates it and saves it. Prefer patch_project for small edits.",
        properties: || json!({ "project": { "type": "object", "description": "A complete project." } }),
        required: &["project"],
    },
    ToolDef {
        name: "validate",
        description: "Validates the saved project. Errors block code generation; warnings do not.",
        properties: no_properties,
        required: &[],
    },
    ToolDef {
        name: "list_generated_files",
        description: "Paths of the files the project generates. Fails while the project has errors.",
        properties: no_properties,
        required: &[],
    },
    ToolDef {
        name: "read_generated_file",
        description: "Content of one generated file, by the path list_generated_files returns.",
        properties: || json!({ "path": { "type": "string" } }),
        required: &["path"],
    },
    ToolDef {
        name: "submit_report",
        description: "Tells the Service Builder maintainers what is missing or could be better (a logic block, a field, a check, a tool). Call it when Service Builder asks for a report, or whenever you hit a real limitation. The user reads the report and decides whether it is published as a GitHub issue, so describe the general need only: no project names, secrets or file contents.",
        properties: || {
            json!({
                "summary": { "type": "string", "description": "One or two sentences of context, optional." },
                "suggestions": {
                    "type": "array",
                    "description": "1 to 5 concrete suggestions.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "description": "What to add, in a few words." },
                            "details": { "type": "string", "description": "Why it helps, and how it could work." }
                        },
                        "required": ["title"]
                    }
                }
            })
        },
        required: &["suggestions"],
    },
    ToolDef {
        name: "export_code",
        description: "Writes the generated Cargo workspace into a directory (empty, or a previous export: its files are overwritten). Fails while the project has errors.",
        properties: || json!({ "dir": { "type": "string", "description": "Absolute path of the destination folder." } }),
        required: &["dir"],
    },
];

pub fn tool_summaries() -> Vec<ToolSummary> {
    TOOLS
        .iter()
        .map(|t| ToolSummary {
            name: t.name,
            description: t.description,
        })
        .collect()
}

fn tools_list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": { "type": "object", "properties": (t.properties)(), "required": t.required },
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// Serves the protocol on `input`/`output` until `input` closes.
pub fn serve(project: &Path, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(project, &message),
            Err(e) => Some(error_reply(
                &Value::Null,
                PARSE_ERROR,
                &format!("parse error: {e}"),
            )),
        };
        if let Some(reply) = reply {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn result_reply(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_reply(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// One message in, at most one reply out. Notifications (no `id`) never get a reply.
pub fn handle(project: &Path, message: &Value) -> Option<Value> {
    let id = message.get("id")?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let reply = match message.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let protocol = asked
                .filter(|p| SUPPORTED_PROTOCOLS.contains(p))
                .unwrap_or(PROTOCOL);
            result_reply(
                id,
                json!({
                    "protocolVersion": protocol,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "service-builder", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Edits a Service Builder project (event-driven Rust backends). Call describe_project first; edit with patch_project; check with validate.",
                }),
            )
        }
        Some("ping") => result_reply(id, json!({})),
        Some("tools/list") => result_reply(id, tools_list()),
        Some("tools/call") => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error_reply(id, INVALID_PARAMS, "missing tool name"));
            };
            if !TOOLS.iter().any(|t| t.name == name) {
                return Some(error_reply(
                    id,
                    INVALID_PARAMS,
                    &format!("unknown tool {name}"),
                ));
            }
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            // A failing tool is a result the model can read and react to, not a protocol error.
            let (text, is_error) = match call_tool(project, name, arguments) {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            let mut content = vec![json!({ "type": "text", "text": text })];
            // Every few calls the agent is asked, in the result itself, for a feedback report.
            if name != "submit_report" && crate::feedback::tick(project) {
                content.push(json!({ "type": "text", "text": crate::feedback::request_text() }));
            }
            result_reply(id, json!({ "content": content, "isError": is_error }))
        }
        Some(other) => error_reply(id, METHOD_NOT_FOUND, &format!("method not found: {other}")),
        None => error_reply(id, METHOD_NOT_FOUND, "missing method"),
    };
    Some(reply)
}

fn call_tool(project: &Path, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "describe_project" => Ok(pretty(&outline(&load_project(project)?))),
        "get_project" => Ok(pretty(&load_project(project)?)),
        "get_schema" => schema_tool(args.get("definition").and_then(Value::as_str)),
        "get_example_project" => Ok(pretty(&sb_model::sample::orders())),
        "patch_project" => {
            let operations = args
                .get("operations")
                .and_then(Value::as_array)
                .ok_or("`operations` must be an array")?;
            // Nothing is saved before every operation went through, so a failure leaves the file as it was.
            let mut document =
                serde_json::to_value(load_project(project)?).map_err(|e| e.to_string())?;
            for (i, operation) in operations.iter().enumerate() {
                apply_operation(&mut document, operation)
                    .map_err(|e| format!("operation {i}: {e}"))?;
            }
            store(project, document)
        }
        "set_project" => store(
            project,
            args.get("project")
                .cloned()
                .ok_or("`project` is required")?,
        ),
        "validate" => Ok(report(&sb_codegen::diagnose(&load_project(project)?))),
        "list_generated_files" => {
            let files = generated(project)?;
            Ok(files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "read_generated_file" => {
            let wanted = string_arg(args, "path")?;
            generated(project)?
                .into_iter()
                .find(|f| f.path == wanted)
                .map(|f| f.content)
                .ok_or_else(|| format!("no generated file {wanted}; see list_generated_files"))
        }
        "submit_report" => crate::feedback::submit(project, args),
        "export_code" => {
            let report =
                export_project(&load_project(project)?, Path::new(string_arg(args, "dir")?))?;
            Ok(format!("wrote {} files to {}", report.files, report.dir))
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

fn schema_tool(definition: Option<&str>) -> Result<String, String> {
    let schema = sb_model::json_schema();
    let definitions = schema
        .get("$defs")
        .and_then(Value::as_object)
        .ok_or("the schema has no definitions")?;
    let Some(name) = definition else {
        let listing: Vec<String> = definitions
            .iter()
            .map(
                |(name, d)| match d.get("description").and_then(Value::as_str) {
                    Some(text) => format!("{name}: {}", first_sentence(text)),
                    None => name.clone(),
                },
            )
            .collect();
        return Ok(format!(
            "Root: Project. Pass one of these as `definition` to get it:\n{}",
            listing.join("\n")
        ));
    };
    if !definitions.contains_key(name) {
        let known: Vec<&str> = definitions.keys().map(String::as_str).collect();
        return Err(format!("no definition {name}; known: {}", known.join(", ")));
    }
    // The definition, then everything it refers to (transitively), so it can be read on its own.
    let mut wanted = vec![name.to_string()];
    let mut at = 0;
    while at < wanted.len() {
        let mut found = Vec::new();
        collect_refs(&definitions[&wanted[at]], &mut found);
        for r in found {
            if !wanted.contains(&r) {
                wanted.push(r);
            }
        }
        at += 1;
    }
    let picked: serde_json::Map<String, Value> = wanted
        .iter()
        .map(|n| (n.clone(), definitions[n].clone()))
        .collect();
    Ok(pretty(&json!({ "definition": name, "$defs": picked })))
}

/// The first sentence of a doc comment, on one line.
fn first_sentence(text: &str) -> String {
    let paragraph = text.split("\n\n").next().unwrap_or("");
    let flat = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.find(". ") {
        Some(end) => flat[..=end].to_string(),
        None => flat,
    }
}

fn collect_refs(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                match (key.as_str(), v.as_str()) {
                    ("$ref", Some(target)) => {
                        if let Some(name) = target.strip_prefix("#/$defs/") {
                            into.push(name.to_string());
                        }
                    }
                    _ => collect_refs(v, into),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_refs(v, into)),
        _ => {}
    }
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` must be a string"))
}

fn pretty(value: &impl Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn generated(project: &Path) -> Result<Vec<sb_codegen::GeneratedFile>, String> {
    let analysis = analyze(&load_project(project)?);
    if analysis.files.is_empty() {
        return Err(format!(
            "the project has errors, nothing is generated:\n{}",
            report(&analysis.diagnostics)
        ));
    }
    Ok(analysis.files)
}

/// Checks that `document` is a project, saves it, and reports what the validator says about it.
fn store(project_file: &Path, document: Value) -> Result<String, String> {
    let mut project: Project =
        serde_json::from_value(document).map_err(|e| format!("not a valid project: {e}"))?;
    project.migrate();
    project.freeze_field_numbers();
    save_project(project_file, &project)?;
    Ok(format!(
        "saved {}\n{}",
        project_file.display(),
        report(&sb_codegen::diagnose(&project))
    ))
}

fn report(diagnostics: &[sb_model::Diagnostic]) -> String {
    let count = |s| diagnostics.iter().filter(|d| d.severity == s).count();
    let mut text = format!(
        "{} error(s), {} warning(s){}",
        count(Severity::Error),
        count(Severity::Warning),
        if has_errors(diagnostics) {
            ": code generation is blocked"
        } else {
            ""
        }
    );
    for d in diagnostics {
        let level = if d.severity == Severity::Error {
            "error"
        } else {
            "warning"
        };
        text.push_str(&format!("\n{level}[{}] {}: {}", d.code, d.path, d.message));
    }
    text
}

fn outline(project: &Project) -> Value {
    let names = |it: Vec<String>| Value::from(it);
    json!({
        "name": project.name,
        "schemaVersion": project.schema_version,
        "messages": project.messages.iter().map(|m| json!({
            "name": m.name,
            "kind": m.kind,
            "fields": m.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "topics": project.topics.iter().map(|t| json!({
            "name": t.name, "message": t.message, "producers": t.producers,
        })).collect::<Vec<_>>(),
        "services": project.services.iter().enumerate().map(|(i, s)| json!({
            "index": i,
            "name": s.name,
            "rpcs": names(s.rpcs.iter().map(|r| format!("{}({}) -> {}", r.name, r.request, r.response)).collect()),
            "routes": names(s.routes.iter().map(|r| format!("{} {} -> {}", serde_json::to_value(r.method).ok().and_then(|m| m.as_str().map(str::to_uppercase)).unwrap_or_default(), r.path, r.name)).collect()),
            "tables": names(s.tables.iter().map(|t| format!("{}({})", t.name, t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "))).collect()),
            "consumers": names(s.consumers.iter().map(|c| format!("{} <- {}", c.name, c.topic)).collect()),
        })).collect::<Vec<_>>(),
    })
}

// ---- JSON Patch (RFC 6902), the three operations an editing client needs ----

fn apply_operation(document: &mut Value, operation: &Value) -> Result<(), String> {
    let op = operation
        .get("op")
        .and_then(Value::as_str)
        .ok_or("missing `op`")?;
    let path = operation
        .get("path")
        .and_then(Value::as_str)
        .ok_or("missing `path`")?;
    let (parent_path, key) = path.rsplit_once('/').ok_or(
        "`path` must be a JSON pointer starting with / (use set_project to replace everything)",
    )?;
    let key = key.replace("~1", "/").replace("~0", "~");
    let parent = document
        .pointer_mut(parent_path)
        .ok_or_else(|| format!("{parent_path} does not exist"))?;
    let value = || {
        operation
            .get("value")
            .cloned()
            .ok_or(format!("`{op}` needs a `value`"))
    };
    match (op, parent) {
        ("add", Value::Object(map)) => {
            map.insert(key, value()?);
        }
        ("add", Value::Array(items)) => {
            if key == "-" {
                items.push(value()?);
            } else {
                let at = index(&key, items.len(), true)?;
                items.insert(at, value()?);
            }
        }
        ("replace", Value::Object(map)) => {
            let slot = map
                .get_mut(&key)
                .ok_or_else(|| format!("{path} does not exist"))?;
            *slot = value()?;
        }
        ("replace", Value::Array(items)) => {
            let at = index(&key, items.len(), false)?;
            items[at] = value()?;
        }
        ("remove", Value::Object(map)) => {
            map.remove(&key)
                .ok_or_else(|| format!("{path} does not exist"))?;
        }
        ("remove", Value::Array(items)) => {
            let at = index(&key, items.len(), false)?;
            items.remove(at);
        }
        ("add" | "replace" | "remove", _) => {
            return Err(format!("{parent_path} is neither an object nor an array"));
        }
        _ => return Err(format!("unsupported op `{op}` (add, replace, remove)")),
    }
    Ok(())
}

fn index(key: &str, len: usize, allow_end: bool) -> Result<usize, String> {
    let at: usize = key
        .parse()
        .map_err(|_| format!("`{key}` is not an array index"))?;
    if at < len || (allow_end && at == len) {
        Ok(at)
    } else {
        Err(format!("index {at} is out of range (length {len})"))
    }
}

// ---- Activation: the entry of the client configuration that starts this server ----

/// Where the project's MCP entry stands in the `.mcp.json` next to the project file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryState {
    /// No entry for this project.
    Off,
    /// The entry starts this very program on this very project.
    On,
    /// An entry exists but starts something else (the app moved or was updated).
    Stale,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activation {
    /// The `.mcp.json` that is read and written.
    pub config_path: String,
    /// Key of the entry in `mcpServers`.
    pub server_name: String,
    pub state: EntryState,
}

/// `.mcp.json` is the project-scoped configuration Claude Code reads from the folder it runs in.
pub fn config_path(project_file: &Path) -> PathBuf {
    project_file
        .parent()
        .unwrap_or(Path::new("."))
        .join(".mcp.json")
}

/// One entry per project file, so two projects in the same folder do not overwrite each other.
pub fn server_name(project_file: &Path) -> String {
    format!("service-builder-{}", project_slug(project_file))
}

/// The project file's name without `.sb.json`, reduced to lowercase letters, digits and dashes.
pub(crate) fn project_slug(project_file: &Path) -> String {
    let file = project_file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    let stem = file
        .strip_suffix(".sb.json")
        .or_else(|| file.rsplit_once('.').map(|(s, _)| s))
        .unwrap_or(file);
    let slug: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    slug.trim_matches('-').to_string()
}

/// What the client runs: this program in `mcp` mode on the project file.
pub fn server_entry(command: &str, project_file: &Path) -> Value {
    json!({
        "type": "stdio",
        "command": command,
        "args": ["mcp", project_file.display().to_string()],
        "env": {},
    })
}

fn read_config(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| {
                format!(
                    "{} is not a JSON object; fix or remove it first",
                    path.display()
                )
            }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn write_config(path: &Path, config: &Value) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, pretty(config) + "\n")
        .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn activation(project_file: &Path, command: &str) -> Result<Activation, String> {
    let path = config_path(project_file);
    let name = server_name(project_file);
    let state = match read_config(&path)?.pointer(&format!("/mcpServers/{name}")) {
        None => EntryState::Off,
        Some(entry) if *entry == server_entry(command, project_file) => EntryState::On,
        Some(_) => EntryState::Stale,
    };
    Ok(Activation {
        config_path: path.display().to_string(),
        server_name: name,
        state,
    })
}

/// Adds (or refreshes) the project's entry, leaving every other entry of the file alone.
pub fn enable(project_file: &Path, command: &str) -> Result<Activation, String> {
    let path = config_path(project_file);
    let mut config = read_config(&path)?;
    let servers = config
        .as_object_mut()
        .expect("read_config returns an object")
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    servers
        .as_object_mut()
        .ok_or_else(|| format!("`mcpServers` in {} is not an object", path.display()))?
        .insert(
            server_name(project_file),
            server_entry(command, project_file),
        );
    write_config(&path, &config)?;
    activation(project_file, command)
}

pub fn disable(project_file: &Path, command: &str) -> Result<Activation, String> {
    let path = config_path(project_file);
    let mut config = read_config(&path)?;
    if let Some(servers) = config.get_mut("mcpServers").and_then(Value::as_object_mut)
        && servers.remove(&server_name(project_file)).is_some()
    {
        write_config(&path, &config)?;
    }
    activation(project_file, command)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        dir: PathBuf,
        file: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("sb-mcp-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let file = dir.join("orders.sb.json");
            save_project(&file, &sb_model::sample::orders()).unwrap();
            Fixture { dir, file }
        }

        fn call(&self, name: &str, args: Value) -> (String, bool) {
            let reply = handle(
                &self.file,
                &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } }),
            )
            .unwrap();
            let result = &reply["result"];
            (
                result["content"][0]["text"].as_str().unwrap().to_string(),
                result["isError"].as_bool().unwrap(),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn initialize_negotiates_the_protocol_and_advertises_tools() {
        let f = Fixture::new("init");
        let reply = handle(&f.file, &json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } })).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        let unknown = handle(&f.file, &json!({ "id": 0, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } })).unwrap();
        assert_eq!(unknown["result"]["protocolVersion"], PROTOCOL);
    }

    #[test]
    fn notifications_get_no_reply_and_unknown_methods_an_error() {
        let f = Fixture::new("notif");
        assert!(
            handle(
                &f.file,
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
            )
            .is_none()
        );
        let reply = handle(&f.file, &json!({ "id": 3, "method": "resources/list" })).unwrap();
        assert_eq!(reply["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(reply["id"], 3);
    }

    #[test]
    fn lists_every_tool_with_an_input_schema() {
        let f = Fixture::new("list");
        let reply = handle(&f.file, &json!({ "id": 1, "method": "tools/list" })).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOLS.len());
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"
            && !t["description"].as_str().unwrap().is_empty()));
    }

    #[test]
    fn serves_lines_over_a_stream() {
        let f = Fixture::new("stream");
        let input = "{\"id\":1,\"method\":\"ping\"}\n\n{\"method\":\"notifications/initialized\"}\nnot json\n";
        let mut out = Vec::new();
        serve(&f.file, input.as_bytes(), &mut out).unwrap();
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["result"], json!({}));
        assert_eq!(lines[1]["error"]["code"], PARSE_ERROR);
    }

    #[test]
    fn describes_and_validates_the_saved_project() {
        let f = Fixture::new("describe");
        let (outline, err) = f.call("describe_project", json!({}));
        assert!(!err);
        assert!(outline.contains("\"Orders\"") && outline.contains("GET /orders/{order_id}"));
        let (report, err) = f.call("validate", json!({}));
        assert!(!err);
        assert!(report.starts_with("0 error(s)"), "{report}");
    }

    #[test]
    fn patch_edits_saves_and_reports_diagnostics() {
        let f = Fixture::new("patch");
        let (text, err) = f.call(
            "patch_project",
            json!({ "operations": [
                { "op": "replace", "path": "/topics/0/name", "value": "orders.accepted" },
            ]}),
        );
        assert!(!err, "{text}");
        assert!(
            load_project(&f.file)
                .unwrap()
                .topics
                .iter()
                .any(|t| t.name == "orders.accepted")
        );
        // A topic renamed by hand leaves dangling references: the validator says so.
        assert!(text.contains("error(s)"), "{text}");
    }

    #[test]
    fn patch_is_all_or_nothing() {
        let f = Fixture::new("atomic");
        let before = fs::read_to_string(&f.file).unwrap();
        let (text, err) = f.call(
            "patch_project",
            json!({ "operations": [
                { "op": "replace", "path": "/name", "value": "Renamed" },
                { "op": "remove", "path": "/services/99" },
            ]}),
        );
        assert!(err);
        assert!(text.contains("operation 1"), "{text}");
        assert_eq!(fs::read_to_string(&f.file).unwrap(), before);
    }

    #[test]
    fn patch_refuses_a_result_that_is_not_a_project() {
        let f = Fixture::new("shape");
        let before = fs::read_to_string(&f.file).unwrap();
        let (text, err) = f.call(
            "patch_project",
            json!({ "operations": [{ "op": "replace", "path": "/services", "value": "oops" }] }),
        );
        assert!(err);
        assert!(text.contains("not a valid project"), "{text}");
        assert_eq!(fs::read_to_string(&f.file).unwrap(), before);
    }

    #[test]
    fn patch_adds_to_arrays_and_escapes_pointers() {
        let mut doc = json!({ "a": [1, 3], "b/c": { "d~e": 1 } });
        apply_operation(
            &mut doc,
            &json!({ "op": "add", "path": "/a/1", "value": 2 }),
        )
        .unwrap();
        apply_operation(
            &mut doc,
            &json!({ "op": "add", "path": "/a/-", "value": 4 }),
        )
        .unwrap();
        apply_operation(
            &mut doc,
            &json!({ "op": "replace", "path": "/b~1c/d~0e", "value": 9 }),
        )
        .unwrap();
        assert_eq!(doc, json!({ "a": [1, 2, 3, 4], "b/c": { "d~e": 9 } }));
        assert!(
            apply_operation(
                &mut doc,
                &json!({ "op": "replace", "path": "/zz", "value": 1 })
            )
            .is_err()
        );
        assert!(
            apply_operation(
                &mut doc,
                &json!({ "op": "add", "path": "/a/9", "value": 1 })
            )
            .is_err()
        );
        assert!(
            apply_operation(&mut doc, &json!({ "op": "move", "path": "/a", "value": 1 })).is_err()
        );
    }

    #[test]
    fn get_schema_lists_definitions_and_returns_one_with_what_it_references() {
        let f = Fixture::new("schema");
        let (list, err) = f.call("get_schema", json!({}));
        assert!(!err);
        assert!(list.contains("\nStep: One block of a handler"), "{list}");
        // Every line is one whole sentence, not a doc comment cut at the line wrap.
        assert!(list.lines().skip(1).all(|l| !l.ends_with(':')), "{list}");

        let (step, err) = f.call("get_schema", json!({ "definition": "Step" }));
        assert!(!err);
        let schema: Value = serde_json::from_str(&step).unwrap();
        let defs = schema["$defs"].as_object().unwrap();
        for name in [
            "Step",
            "Expr",
            "Condition",
            "CmpOp",
            "SqlOp",
            "SqlCondition",
        ] {
            assert!(
                defs.contains_key(name),
                "{name} missing from {:?}",
                defs.keys().collect::<Vec<_>>()
            );
        }
        assert!(!defs.contains_key("Project"));
        assert!(step.contains("\"sql\"") && step.contains("\"branch\""));

        let (text, err) = f.call("get_schema", json!({ "definition": "Nope" }));
        assert!(err);
        assert!(text.contains("known:") && text.contains("Step"), "{text}");
    }

    #[test]
    fn set_project_replaces_the_file() {
        let f = Fixture::new("set");
        let empty = serde_json::to_value(Project::new("Vide")).unwrap();
        let (text, err) = f.call("set_project", json!({ "project": empty }));
        assert!(!err, "{text}");
        assert_eq!(load_project(&f.file).unwrap().name, "Vide");
    }

    #[test]
    fn generated_files_can_be_listed_read_and_exported() {
        let f = Fixture::new("gen");
        let (list, err) = f.call("list_generated_files", json!({}));
        assert!(!err);
        assert!(list.lines().any(|l| l == "contracts/src/lib.rs"), "{list}");
        let (content, err) = f.call(
            "read_generated_file",
            json!({ "path": "contracts/src/lib.rs" }),
        );
        assert!(!err && content.contains("pub"), "{content}");
        assert!(
            f.call("read_generated_file", json!({ "path": "nope.rs" }))
                .1
        );

        let out = f.dir.join("out");
        let (text, err) = f.call("export_code", json!({ "dir": out.display().to_string() }));
        assert!(!err, "{text}");
        assert!(out.join("Cargo.toml").exists());
    }

    #[test]
    fn generation_tools_refuse_a_project_with_errors() {
        let f = Fixture::new("blocked");
        f.call("patch_project", json!({ "operations": [{ "op": "replace", "path": "/topics/0/message", "value": "Nope" }] }));
        let (text, err) = f.call("list_generated_files", json!({}));
        assert!(err);
        assert!(text.contains("has errors"), "{text}");
    }

    #[test]
    fn unknown_tool_is_a_protocol_error_and_a_missing_file_a_tool_error() {
        let f = Fixture::new("unknown");
        let reply = handle(
            &f.file,
            &json!({ "id": 1, "method": "tools/call", "params": { "name": "nope" } }),
        )
        .unwrap();
        assert_eq!(reply["error"]["code"], INVALID_PARAMS);
        fs::remove_file(&f.file).unwrap();
        let (text, err) = f.call("get_project", json!({}));
        assert!(err);
        assert!(text.contains("cannot read"), "{text}");
    }

    #[test]
    fn the_fiftieth_call_asks_the_agent_for_a_report_and_submit_report_files_it() {
        let f = Fixture::new("report");
        let answer = |f: &Fixture| {
            handle(&f.file, &json!({ "id": 1, "method": "tools/call", "params": { "name": "describe_project", "arguments": {} } })).unwrap()
        };
        for call in 1..crate::feedback::REPORT_EVERY {
            let reply = answer(&f);
            assert_eq!(
                reply["result"]["content"].as_array().unwrap().len(),
                1,
                "call {call}"
            );
        }
        let fiftieth = answer(&f);
        let content = fiftieth["result"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert!(
            content[1]["text"]
                .as_str()
                .unwrap()
                .contains("submit_report")
        );

        let (text, err) = f.call(
            "submit_report",
            json!({ "suggestions": [{ "title": "Blocking task block", "details": "spawn_blocking for password hashing" }] }),
        );
        assert!(!err, "{text}");
        let feedback = crate::feedback::load(&f.file).unwrap();
        assert_eq!(feedback.reports.len(), 1);
        // Answering the request does not count as an interaction, and the next request is 50 calls away.
        assert_eq!(feedback.interactions, 0);
        assert_eq!(answer(&f)["result"]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_invalid_report_is_a_tool_error_the_agent_can_fix() {
        let f = Fixture::new("badreport");
        let (text, err) = f.call("submit_report", json!({ "suggestions": [] }));
        assert!(err);
        assert!(text.contains("suggestions"), "{text}");
    }

    #[test]
    fn entries_are_named_after_the_project_file() {
        assert_eq!(
            server_name(Path::new("/w/Mon Projet.sb.json")),
            "service-builder-mon-projet"
        );
        assert_eq!(
            server_name(Path::new("/w/demo.json")),
            "service-builder-demo"
        );
    }

    #[test]
    fn activation_cycle_keeps_other_servers_and_detects_a_moved_app() {
        let f = Fixture::new("activation");
        let path = config_path(&f.file);
        fs::write(
            &path,
            r#"{ "mcpServers": { "other": { "command": "x" } }, "keep": 1 }"#,
        )
        .unwrap();

        assert_eq!(
            activation(&f.file, "/opt/sb").unwrap().state,
            EntryState::Off
        );
        assert_eq!(enable(&f.file, "/opt/sb").unwrap().state, EntryState::On);
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["keep"], 1);
        assert_eq!(written["mcpServers"]["other"]["command"], "x");
        let entry = &written["mcpServers"]["service-builder-orders"];
        assert_eq!(entry["command"], "/opt/sb");
        assert_eq!(entry["args"], json!(["mcp", f.file.display().to_string()]));

        assert_eq!(
            activation(&f.file, "/moved/sb").unwrap().state,
            EntryState::Stale
        );
        assert_eq!(enable(&f.file, "/moved/sb").unwrap().state, EntryState::On);

        assert_eq!(
            disable(&f.file, "/moved/sb").unwrap().state,
            EntryState::Off
        );
        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["mcpServers"], json!({ "other": { "command": "x" } }));
    }

    #[test]
    fn activation_creates_the_file_and_refuses_to_clobber_garbage() {
        let f = Fixture::new("garbage");
        assert_eq!(enable(&f.file, "/opt/sb").unwrap().state, EntryState::On);
        fs::write(config_path(&f.file), "{ not json").unwrap();
        assert!(
            enable(&f.file, "/opt/sb")
                .unwrap_err()
                .contains("not a JSON object")
        );
        assert_eq!(
            fs::read_to_string(config_path(&f.file)).unwrap(),
            "{ not json"
        );
    }
}
