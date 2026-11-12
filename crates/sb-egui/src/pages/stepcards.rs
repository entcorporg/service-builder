//! The steps of a flow as cards: a tinted icon, the name of the step and what it does, and a body that opens in place
//! with the fields of the step. A condition holds two more lists of cards, `alors` and `sinon`.

use eframe::egui::{
    self, Color32, CornerRadius, Frame, Margin, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui,
    UiBuilder, Vec2,
};
use sb_app::scope::Var;
use sb_app::stepform::{self, Kind};
use sb_model::{Assignment, CmpOp, Condition, Expr, Field, FieldType, Project, Step};

use super::assignments::{assignments, expr_editor};
use super::common::{commit_entry, icon_btn};
use super::steps::{Flow, Loc, list_mut, new_id, step_move, vars_at};
use crate::app::App;
use crate::fonts::Weight;
use crate::icons;
use crate::theme::*;
use crate::widgets;

/// The name of a kind of step, as the web version writes it.
pub fn title(kind: &str) -> &'static str {
    match kind {
        "log" => "Journal",
        "let" => "Variable",
        "publish" => "Publier un événement",
        "publishLive" => "Pousser vers les navigateurs",
        "callRpc" => "Appeler un RPC",
        "branch" => "Condition",
        "respond" => "Répondre",
        "signJwt" => "Signer un jeton",
        "verifyJwt" => "Vérifier un jeton",
        "jwks" => "Publier les clés",
        "totpGenerate" => "Activer le 2FA",
        "totpVerify" => "Vérifier un code 2FA",
        "recoveryCodes" => "Codes de récupération",
        "recoveryCodeUse" => "Utiliser un code de récupération",
        "passkeyRegisterStart" => "Passkey : début d’enregistrement",
        "passkeyRegisterFinish" => "Passkey : fin d’enregistrement",
        "passkeyLoginStart" => "Passkey : début de connexion",
        "passkeyLoginFinish" => "Passkey : fin de connexion",
        "imageSniff" => "Reconnaître une image",
        "objectPut" => "Stocker un objet",
        "objectGet" => "Lire un objet",
        "objectPresignGet" => "URL de lecture signée",
        "objectPresignPut" => "URL d’envoi signée",
        "objectDelete" => "Supprimer un objet",
        "encrypt" => "Chiffrer",
        "decrypt" => "Déchiffrer",
        "verifyHmac" => "Vérifier une signature",
        "rateLimit" => "Limiter le débit",
        "hashPassword" => "Hacher un mot de passe",
        "verifyPassword" => "Vérifier un mot de passe",
        "randomToken" => "Jeton aléatoire",
        "sha256" => "Empreinte SHA-256",
        "fail" => "Refuser",
        "sql" => "SQL",
        "code" => "Code Rust",
        _ => "Étape",
    }
}

/// The tint of the icon of a kind of step: (background, alpha, colour of the icon).
fn tone(kind: &str) -> (Color32, u8, Color32) {
    let (bg, a, fg) = match kind {
        "log" => (0x8f97bd, 46, 0xb6bdea),
        "let" => (0xb482ff, 46, 0xc7a0ff),
        "publish" | "publishLive" => (0xffb04a, 46, 0xffb04a),
        "callRpc" | "sql" => (0x7886ff, 51, 0x7886ff),
        "branch" => (0xff66a0, 43, 0xff8fb8),
        "respond" => (0x3ecf8e, 43, 0x3ecf8e),
        "fail" => (0xff6678, 38, 0xff6678),
        "rateLimit" => (0xff8c5a, 41, 0xff9c6e),
        "code" => (0x78dcff, 38, 0x7fdcff),
        "imageSniff" | "objectPut" | "objectGet" | "objectDelete" | "objectPresignGet"
        | "objectPresignPut" => (0x4ecdc4, 38, 0x4ecdc4),
        _ => (0xffc44d, 38, 0xffc44d),
    };
    (hex(bg), a, hex(fg))
}

/// One line that says what a step does, in words.
fn summary(step: &Step, kind: &str) -> String {
    let texts = stepform::to_texts(step);
    let get = |k: &str| {
        texts
            .iter()
            .find(|(s, _)| s.key == k)
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    };
    match kind {
        "rateLimit" => format!("{} par {} s", get("max"), get("windowSecs")),
        "branch" => {
            let c = get("condition");
            let c = c
                .replace("<=", "≤")
                .replace(">=", "≥")
                .replace("!=", "≠")
                .replace("==", "=");
            format!("si {c}")
        }
        "fail" => format!("{} {}", get("status"), get("message"))
            .trim()
            .to_string(),
        "let" => format!("{} : {}", get("name"), get("valueType")),
        "log" => get("message"),
        _ => {
            let full = stepform::summary(step);
            full.split_once("  ")
                .map_or(String::new(), |(_, d)| d.to_string())
        }
    }
}

/// The name of a field of a step, as a person reads it.
fn field_name(key: &str) -> String {
    let known = match key {
        "message" => "Message",
        "status" => "Statut",
        "retryAfter" => "Réessayer après (s)",
        "name" => "Nom",
        "optional" => "Optionnel",
        "repeated" => "Liste",
        "value" => "Valeur",
        "valueType" => "Type",
        "condition" => "Si",
        "key" => "Clé",
        "max" => "Maximum",
        "windowSecs" => "Fenêtre (s)",
        "topic" => "Topic",
        "payload" => "Contenu",
        "table" => "Table",
        "op" => "Opération",
        "fields" => "Champs",
        "level" => "Niveau",
        "service" => "Service",
        "rpc" => "RPC",
        "args" => "Arguments",
        "into" => "Résultat dans",
        "values" => "Valeurs",
        "set" => "Modifications",
        "filters" | "where" => "Conditions",
        "code" => "Code Rust",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

const FAIL_STATUSES: [(u16, &str); 9] = [
    (400, "requête invalide"),
    (401, "non authentifié"),
    (403, "interdit"),
    (404, "introuvable"),
    (409, "conflit"),
    (422, "entité invalide"),
    (429, "trop de requêtes"),
    (500, "erreur interne"),
    (503, "indisponible"),
];

fn as_expr(text: &str) -> Option<Expr> {
    stepform::parse_expr(text)
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
}

fn hint_line(ui: &mut Ui, text: &str) {
    if !text.is_empty() {
        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(FAINT)).wrap());
    }
}

fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(FAINT));
}

fn kind_of(step: &Step) -> String {
    serde_json::to_value(step)
        .ok()
        .and_then(|v| v["type"].as_str().map(String::from))
        .unwrap_or_default()
}

const OPS: [(&str, CmpOp); 6] = [
    ("=", CmpOp::Eq),
    ("≠", CmpOp::Ne),
    (">", CmpOp::Gt),
    ("≥", CmpOp::Ge),
    ("<", CmpOp::Lt),
    ("≤", CmpOp::Le),
];

fn default_compare() -> Condition {
    Condition::Compare {
        left: Expr::Path {
            path: String::new(),
        },
        op: CmpOp::Eq,
        right: Expr::Literal {
            value: serde_json::Value::from(0),
        },
    }
}

/// `Si` a comparison of two values, or a Rust condition. Returns the new text when it changed.
fn condition_widget(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    text: &str,
    vars: &[Var],
) -> Option<String> {
    let start = stepform::parse_condition(text)
        .ok()
        .and_then(|v| serde_json::from_value::<Condition>(v).ok())
        .unwrap_or_else(default_compare);
    let mut next: Option<Condition> = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            field_label(ui, "Si");
            let kinds = ["comparaison".to_string(), "Rust".to_string()];
            let now = if matches!(start, Condition::Raw { .. }) {
                "Rust"
            } else {
                "comparaison"
            };
            if let Some(m) = super::assignments::pick(ui, id.with("mode"), now, &kinds, f32::MAX)
                && m != now
            {
                next = Some(if m == "Rust" {
                    Condition::Raw {
                        code: String::new(),
                    }
                } else {
                    default_compare()
                });
            }
        });
        match &start {
            Condition::Compare { left, op, right } => {
                if let Some(Some(e)) =
                    expr_editor(app, ui, id.with("left"), None, false, Some(left), vars)
                {
                    next = Some(Condition::Compare {
                        left: e,
                        op: *op,
                        right: right.clone(),
                    });
                }
                let names: Vec<String> = OPS.iter().map(|(n, _)| (*n).to_string()).collect();
                let shown = OPS.iter().find(|(_, o)| o == op).map_or("=", |(n, _)| *n);
                if let Some(v) = super::assignments::pick(ui, id.with("op"), shown, &names, 60.0)
                    && let Some((_, o)) = OPS.iter().find(|(n, _)| *n == v)
                {
                    next = Some(Condition::Compare {
                        left: left.clone(),
                        op: *o,
                        right: right.clone(),
                    });
                }
                if let Some(Some(e)) =
                    expr_editor(app, ui, id.with("right"), None, false, Some(right), vars)
                {
                    next = Some(Condition::Compare {
                        left: left.clone(),
                        op: *op,
                        right: e,
                    });
                }
            }
            Condition::Raw { code } => {
                if let Some(v) = commit_entry(
                    ui,
                    id.with("raw"),
                    code,
                    true,
                    "condition Rust, par exemple input.amount > 0",
                    true,
                ) {
                    next = Some(Condition::Raw { code: v });
                }
            }
        }
    });
    next.and_then(|c| serde_json::to_value(&c).ok())
        .map(|v| stepform::condition_to_text(&v))
}

/// The message an assignment list fills, when it is known: the payload of a topic, the request of an RPC, the response.
fn assignment_target(p: &Project, step: &Step, key: &str, loc: &Loc) -> Option<String> {
    let v = serde_json::to_value(step).ok()?;
    match (v["type"].as_str()?, key) {
        ("publish", "payload") => p.topic(v["topic"].as_str()?).map(|t| t.message.clone()),
        ("callRpc", "request") => p
            .service(v["service"].as_str()?)?
            .rpcs
            .iter()
            .find(|r| Some(r.name.as_str()) == v["method"].as_str())
            .map(|r| r.request.clone()),
        ("respond", "fields") => {
            let svc = p.services.get(loc.service)?;
            match loc.flow {
                Flow::Route(i) => svc.routes.get(i).map(|r| r.response.clone()),
                Flow::Rpc(i) => svc.rpcs.get(i).map(|r| r.response.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The type a `let` gives its value, to offer the fields that fit.
fn let_target(step: &Step) -> Option<Field> {
    let v = serde_json::to_value(step).ok()?;
    if v["type"] != "let" {
        return None;
    }
    let ty: FieldType = serde_json::from_value(v["valueType"].clone()).ok()?;
    let mut f = Field::new("", ty);
    f.repeated = v["repeated"].as_bool().unwrap_or(false);
    f.optional = v["optional"].as_bool().unwrap_or(false) && !f.repeated;
    Some(f)
}

/// The fields of a step, as inputs. Any change gives the new step back, or says what is wrong.
fn form(app: &mut App, ui: &mut Ui, step: &Step, loc: &Loc, index: usize) {
    let all = stepform::to_texts(step);
    let mut texts: Vec<String> = all.iter().map(|(_, t)| t.clone()).collect();
    let p = app.project();
    let vars = vars_at(app, loc, index);
    let target = let_target(step);
    let is_fail = matches!(step, Step::Fail { .. });
    let fail_status: String = all
        .iter()
        .find(|(s, _)| s.key == "status")
        .map(|(_, t)| t.clone())
        .unwrap_or_default();
    let mut pending: Option<(usize, String)> = None;
    ui.spacing_mut().item_spacing.y = 8.0;
    for (i, (spec, text)) in all.iter().enumerate() {
        // The `Retry-After` header only means something for 429 and 503.
        if is_fail && spec.key == "retryAfter" && !matches!(fail_status.as_str(), "429" | "503") {
            continue;
        }
        let fid = ui.make_persistent_id(("step-field", step.id(), &spec.key));
        let name = field_name(&spec.key);
        // A structure is edited with widgets (and a schema picked in a list), never typed as JSON. The assignments of a
        // schema keep their own editor.
        let by_schema = matches!(spec.kind, Kind::Assignments)
            && assignment_target(&p, step, &spec.key, loc).is_some();
        if !by_schema
            && let Some(done) = super::stepjson::structured(
                app,
                ui,
                fid,
                &name,
                step,
                loc.service,
                spec,
                text,
                &vars,
            )
        {
            if let Some(v) = done {
                pending = Some((i, v));
            }
            hint_line(ui, &spec.help);
            continue;
        }
        match &spec.kind {
            Kind::Bool => {
                if let Some(on) = widgets::check(ui, text == "true", &name) {
                    pending = Some((i, on.to_string()));
                }
                hint_line(ui, &spec.help);
                continue;
            }
            Kind::Condition => {
                if let Some(v) = condition_widget(app, ui, fid, text, &vars) {
                    pending = Some((i, v));
                }
                hint_line(ui, &spec.help);
                continue;
            }
            _ => {}
        }
        field_label(ui, &name);
        match &spec.kind {
            Kind::Number if is_fail && spec.key == "status" => {
                let words: Vec<String> = FAIL_STATUSES
                    .iter()
                    .map(|(c, w)| format!("{c} · {w}"))
                    .collect();
                let now = FAIL_STATUSES
                    .iter()
                    .find(|(c, _)| c.to_string() == *text)
                    .map_or(text.clone(), |(c, w)| format!("{c} · {w}"));
                if let Some(v) = super::assignments::pick(ui, fid, &now, &words, f32::MAX) {
                    pending = Some((i, v.split(' ').next().unwrap_or("400").to_string()));
                }
            }
            Kind::Choice(words) => {
                if let Some(v) = super::assignments::pick(ui, fid, text, words, f32::MAX) {
                    pending = Some((i, v));
                }
            }
            Kind::Type => {
                let mut options: Vec<String> =
                    stepform::SCALARS.iter().map(|s| (*s).to_string()).collect();
                options.extend(p.messages.iter().map(|m| m.name.clone()));
                if let Some(v) = super::assignments::pick(ui, fid, text, &options, f32::MAX) {
                    pending = Some((i, v));
                }
            }
            Kind::Expr => {
                let current = as_expr(text);
                let target = target.as_ref().filter(|_| spec.key == "value");
                if let Some(e) =
                    expr_editor(app, ui, fid, target, spec.optional, current.as_ref(), &vars)
                {
                    pending = Some((
                        i,
                        e.and_then(|e| serde_json::to_value(e).ok())
                            .map(|v| stepform::expr_to_text(&v))
                            .unwrap_or_default(),
                    ));
                }
            }
            Kind::Assignments => {
                // Without a schema, a list is a list of names and values (handled above, `stepjson`).
                if let Some(message) = assignment_target(&p, step, &spec.key, loc) {
                    let list: Vec<Assignment> = stepform::parse_assignments(text)
                        .ok()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    if let Some(change) = assignments(app, ui, fid, &message, &list, &vars) {
                        let mut next = list.clone();
                        change(&mut next);
                        pending = Some((
                            i,
                            stepform::assignments_to_text(
                                &serde_json::to_value(&next).unwrap_or_default(),
                            ),
                        ));
                    }
                }
            }
            // Never typed by hand: a field of this shape needs its own widgets (`stepjson`) or a schema.
            Kind::Json => hint_line(
                ui,
                "Ce champ se règle par un schéma ou dans le fichier du projet, pas en JSON saisi à la main.",
            ),
            Kind::Number | Kind::Text | Kind::Words => {
                if let Some(v) =
                    commit_entry(ui, fid, text, !matches!(spec.kind, Kind::Text), "", true)
                {
                    pending = Some((i, v));
                }
            }
            Kind::Bool | Kind::Condition => {}
        }
        hint_line(ui, &spec.help);
    }
    if let Some((i, v)) = pending {
        texts[i] = v;
        match stepform::from_texts(step, &texts) {
            Ok(new) => {
                let loc = loc.clone();
                app.edit(move |p| {
                    if let Some(slot) = list_mut(p, &loc).and_then(|l| l.get_mut(index)) {
                        *slot = new;
                    }
                });
            }
            Err(e) => app.say(&e),
        }
    }
}

/// The blocks that can be added, in the order of the web version.
const ORDER: [&str; 35] = [
    "publish",
    "publishLive",
    "sql",
    "callRpc",
    "branch",
    "let",
    "log",
    "respond",
    "fail",
    "rateLimit",
    "verifyHmac",
    "signJwt",
    "verifyJwt",
    "jwks",
    "totpGenerate",
    "totpVerify",
    "recoveryCodes",
    "recoveryCodeUse",
    "passkeyRegisterStart",
    "passkeyRegisterFinish",
    "passkeyLoginStart",
    "passkeyLoginFinish",
    "imageSniff",
    "objectPut",
    "objectGet",
    "objectPresignGet",
    "objectPresignPut",
    "objectDelete",
    "encrypt",
    "decrypt",
    "hashPassword",
    "verifyPassword",
    "randomToken",
    "sha256",
    "code",
];

/// What a block does, under its name in the list of blocks.
fn hint(kind: &str) -> &'static str {
    match kind {
        "log" => "Écrire une ligne de log",
        "let" => "Calculer et nommer une valeur",
        "publish" => "Mis en file (outbox) avec la transaction, puis envoyé sur le topic",
        "publishLive" => {
            "Événement temps réel (SSE), envoyé après le commit, à un compte ou à une permission"
        }
        "callRpc" => "Appeler un autre service en gRPC",
        "branch" => "Si … alors … sinon …",
        "respond" => "Renvoyer la réponse du RPC",
        "signJwt" => {
            "Access token Ed25519 (EdDSA) : vos claims, plus émetteur, émission et expiration"
        }
        "verifyJwt" => {
            "Signature, émetteur et expiration ; les claims lus comme un schéma. Sinon 401"
        }
        "jwks" => {
            "Le JWKS : les clés publiques qui permettent de vérifier un jeton, jamais la graine"
        }
        "totpGenerate" => "Secret TOTP chiffré à stocker, URI otpauth:// pour le QR code",
        "totpVerify" => "Fenêtre ±30 s, un code ne sert qu’une fois (anti-rejeu). Sinon 401",
        "recoveryCodes" => {
            "XXXXX-XXXXX, à montrer une fois ; seules leurs empreintes sont stockées"
        }
        "recoveryCodeUse" => "Valable une seule fois, avec ou sans tiret. Sinon 401",
        "passkeyRegisterStart" => {
            "Un défi pour le navigateur (WebAuthn) ; le compte est déjà authentifié"
        }
        "passkeyRegisterFinish" => {
            "Vérifie la réponse de l’appareil, une seule fois, et stocke la passkey. Sinon 400"
        }
        "passkeyLoginStart" => {
            "Un défi pour les passkeys du compte, ou sans compte : l’appareil propose les siennes. Sans passkey : 401"
        }
        "passkeyLoginFinish" => "Vérifie la réponse et donne le compte. Sinon 401",
        "imageSniff" => {
            "PNG ou JPEG d’après les premiers octets, jamais le Content-Type du client ; pas de SVG. Sinon 400"
        }
        "objectPut" => {
            "Dans un bucket privé (S3 : Garage, MinIO ou AWS). L’objet d’abord, la base ensuite"
        }
        "objectGet" => "En base64 ; rien s’il n’existe pas. Stockage injoignable : 503",
        "objectPresignGet" => {
            "Le front lit l’objet directement (stockage ou CDN) : rien ne transite par l’API. Durée limitée"
        }
        "objectPresignPut" => {
            "Le front envoie le fichier directement au stockage : type et taille sont signés. Durée limitée"
        }
        "objectDelete" => "La base d’abord, l’objet ensuite. Un objet absent n’est pas une erreur",
        "encrypt" => "AES-256-GCM, pour ce qui est stocké (un secret TOTP, un jeton)",
        "decrypt" => "Le texte qu’un bloc « Chiffrer » a scellé",
        "verifyHmac" => {
            "Webhook signé (HMAC-SHA256 du corps brut), sans session : toujours la première étape"
        }
        "rateLimit" => "Au-delà de N essais par fenêtre : 429 et Retry-After",
        "hashPassword" => "argon2id, hors du runtime async : le texte à stocker",
        "verifyPassword" => "Vrai ou faux ; un compte inconnu coûte le même temps",
        "randomToken" => "256 bits en hexadécimal : session, e-mail, réinitialisation",
        "sha256" => "Ce qu’on stocke d’un jeton, en hexadécimal",
        "fail" => "Arrêter la route avec un statut d’erreur HTTP (400, 401, 404, 429…)",
        "sql" => "Lire, insérer, modifier ou supprimer dans une table du service",
        "code" => "Insérer du Rust écrit à la main",
        _ => "",
    }
}

/// The blocks that make sense at this place: a response only where there is one, the errors of HTTP on a route…
fn allowed(app: &App, list: &[Step], loc: &Loc) -> Vec<&'static str> {
    let has_response = matches!(loc.flow, Flow::Route(_) | Flow::Rpc(_));
    let http = matches!(loc.flow, Flow::Route(_));
    let has_signature = list.iter().any(|x| matches!(x, Step::VerifyHmac { .. }));
    let has_realtime = app
        .project()
        .services
        .iter()
        .any(|x| x.kind == sb_model::ServiceKind::Realtime);
    // The signature is checked before the body is read: the first step of the route itself, once.
    let allow_signature = http && loc.nest.is_empty() && !has_signature;
    ORDER
        .into_iter()
        .filter(|t| match *t {
            "publishLive" => has_realtime,
            "respond" => has_response,
            "fail" | "rateLimit" | "verifyJwt" | "imageSniff" | "totpVerify"
            | "recoveryCodeUse" => http,
            t if t.starts_with("passkey") => http,
            "verifyHmac" => allow_signature,
            _ => true,
        })
        .collect()
}

/// Adds a block of this kind at the end of the list, open so that it can be filled in place.
fn add_kind(app: &mut App, loc: &Loc, kind: &str) {
    let id = new_id();
    match stepform::blank(kind, &id) {
        Ok(step) => {
            app.open.insert(format!("step/{id}"));
            let (loc, first) = (loc.clone(), kind == "verifyHmac");
            app.edit(move |p| {
                if let Some(list) = list_mut(p, &loc) {
                    if first {
                        list.insert(0, step);
                    } else {
                        list.push(step);
                    }
                }
            });
        }
        Err(e) => app.say(&e),
    }
}

/// A rounded rectangle as a closed path, for the dashed outline.
fn outline(rect: Rect, r: f32) -> Vec<Pos2> {
    let mut points = vec![];
    let corners = [
        (rect.right_top() + Vec2::new(-r, r), -90.0_f32),
        (rect.right_bottom() + Vec2::new(-r, -r), 0.0),
        (rect.left_bottom() + Vec2::new(r, -r), 90.0),
        (rect.left_top() + Vec2::new(r, r), 180.0),
    ];
    for (c, start) in corners {
        for k in 0..=6 {
            let a = (start + 15.0 * k as f32).to_radians();
            points.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    }
    points.push(points[0]);
    points
}

/// A key for a list of steps, to remember whether the picker under it is open.
fn loc_key(loc: &Loc) -> String {
    let flow = match loc.flow {
        Flow::Rpc(i) => format!("rpc{i}"),
        Flow::Route(i) => format!("route{i}"),
        Flow::Consumer(i) => format!("consumer{i}"),
        Flow::Job(i) => format!("job{i}"),
    };
    let nest: Vec<String> = loc
        .nest
        .iter()
        .map(|(o, i)| format!("{}{i}", if *o { "o" } else { "t" }))
        .collect();
    format!("{}/{flow}/{}", loc.service, nest.join("."))
}

/// The dashed button, and under it, once opened, every block in a grid of two columns (no dialog).
fn dashed_add(app: &mut App, ui: &mut Ui, list: &[Step], loc: &Loc) {
    let key = ui.make_persistent_id(("add-step", loc_key(loc)));
    // `SB_OPEN_ADD` (screenshots): the grid of blocks open.
    let mut open = ui
        .data(|d| d.get_temp::<bool>(key))
        .unwrap_or_else(|| std::env::var_os("SB_OPEN_ADD").is_some());
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    let (line, text) = if r.hovered() {
        (ACCENT, ACCENT)
    } else {
        (BORDER_STRONG, MUTED)
    };
    if r.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), alpha(ACCENT, 15));
    }
    ui.painter().extend(Shape::dashed_line(
        &outline(rect.shrink(0.5), 10.0),
        Stroke::new(1.0, line),
        4.0,
        3.0,
    ));
    let galley = ui
        .painter()
        .layout_no_wrap("Ajouter une étape".into(), prop(BASE), text);
    let total = 14.0 + 6.0 + galley.size().x;
    let left = rect.center().x - total / 2.0;
    icons::paint(
        ui,
        "plus",
        Rect::from_center_size(egui::pos2(left + 7.0, rect.center().y), Vec2::splat(14.0)),
        text,
    );
    ui.painter().galley(
        egui::pos2(left + 20.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text,
    );
    if r.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(key, open));
    }
    if !open {
        return;
    }
    let kinds = allowed(app, list, loc);
    let gap = 6.0;
    let cell_w = (ui.available_width() - gap) / 2.0;
    for pair in kinds.chunks(2) {
        // Both cells of a row have the height of the taller one.
        let cells: Vec<_> = pair
            .iter()
            .map(|kind| {
                let wrap = cell_w - 20.0 - 24.0 - 8.0;
                let name = ui
                    .painter()
                    .layout(title(kind).into(), prop(BASE), TEXT, wrap);
                let what = ui
                    .painter()
                    .layout(hint(kind).into(), prop(XS), MUTED, wrap);
                (*kind, name, what)
            })
            .collect();
        let h = cells
            .iter()
            .map(|(_, n, w)| (n.size().y + w.size().y).max(24.0) + 16.0)
            .fold(0.0, f32::max);
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
        for (n, (kind, name, what)) in cells.into_iter().enumerate() {
            let cell = Rect::from_min_size(
                row.min + Vec2::new(n as f32 * (cell_w + gap), 0.0),
                Vec2::new(cell_w, h),
            );
            let r = ui
                .interact(cell, key.with(("cell", kind)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let hot = r.hovered();
            ui.painter().rect(
                cell,
                CornerRadius::same(9),
                if hot { RAISED } else { PANEL2 },
                Stroke::new(1.0, if hot { ACCENT } else { BORDER }),
                egui::StrokeKind::Inside,
            );
            let (_, _, color) = tone(kind);
            icons::paint(
                ui,
                kind,
                Rect::from_center_size(
                    egui::pos2(cell.left() + 10.0 + 12.0, cell.center().y),
                    Vec2::splat(18.0),
                ),
                color,
            );
            let x = cell.left() + 10.0 + 24.0 + 8.0;
            let y = cell.center().y - (name.size().y + what.size().y) / 2.0;
            let below = y + name.size().y;
            ui.painter().galley(egui::pos2(x, y), name, TEXT);
            ui.painter().galley(egui::pos2(x, below), what, MUTED);
            if r.clicked() {
                add_kind(app, loc, kind);
            }
        }
    }
}

fn step_card(app: &mut App, ui: &mut Ui, step: &Step, loc: &Loc, i: usize, len: usize) {
    let kind = kind_of(step);
    let (bg, bg_alpha, color) = tone(&kind);
    let id = format!("step/{}", step.id());
    let open = app.open.contains(&id) || std::env::var_os("SB_OPEN_ALL").is_some();
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            // The head: chevron, tinted icon, name, summary; up, down and delete on the right.
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
            let buttons = usize::from(i > 0) + usize::from(i + 1 < len) + 1;
            let toggle_rect = Rect::from_min_max(
                rect.min,
                egui::pos2(rect.right() - 4.0 - 26.0 * buttons as f32 - 2.0, rect.max.y),
            );
            let toggle = ui
                .interact(
                    toggle_rect,
                    ui.make_persistent_id(("toggle", &id)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if toggle.hovered() {
                ui.painter().rect_filled(
                    toggle_rect,
                    CornerRadius {
                        nw: 10,
                        sw: if open { 0 } else { 10 },
                        ne: 0,
                        se: 0,
                    },
                    alpha(RAISED, 100),
                );
            }
            let cy = rect.center().y;
            icons::paint(
                ui,
                if open { "down" } else { "right" },
                Rect::from_center_size(egui::pos2(rect.left() + 15.0, cy), Vec2::splat(14.0)),
                FAINT,
            );
            let badge =
                Rect::from_center_size(egui::pos2(rect.left() + 42.0, cy), Vec2::splat(24.0));
            ui.painter()
                .rect_filled(badge, CornerRadius::same(7), alpha(bg, bg_alpha));
            icons::paint(
                ui,
                &kind,
                Rect::from_center_size(badge.center(), Vec2::splat(14.0)),
                color,
            );
            let name = ui
                .painter()
                .layout_no_wrap(title(&kind).into(), prop(BASE), TEXT);
            let x = badge.right() + 8.0;
            let name_w = name.size().x;
            ui.painter()
                .galley(egui::pos2(x, cy - name.size().y / 2.0), name, TEXT);
            let sx = x + name_w + 8.0;
            let room = toggle_rect.right() - 8.0 - sx;
            if room > 20.0 {
                let mut job = egui::text::LayoutJob::single_section(
                    summary(step, &kind),
                    egui::TextFormat::simple(prop(SM), MUTED),
                );
                job.wrap = egui::text::TextWrapping {
                    max_width: room,
                    max_rows: 1,
                    break_anywhere: true,
                    overflow_character: Some('…'),
                };
                let g = ui.painter().layout_job(job);
                ui.painter()
                    .galley(egui::pos2(sx, cy - g.size().y / 2.0), g, MUTED);
            }
            if toggle.clicked() {
                if open {
                    app.open.remove(&id);
                } else {
                    app.open.insert(id.clone());
                }
            }
            // The buttons, from the right: delete, down, up.
            let mut right = rect.right() - 4.0;
            let mut button = |ui: &mut Ui, icon: &str, tip: &str, danger: bool| {
                let r = Rect::from_min_size(egui::pos2(right - 26.0, cy - 13.0), Vec2::splat(26.0));
                right -= 26.0;
                ui.scope_builder(UiBuilder::new().max_rect(r), |ui| {
                    icon_btn(ui, icon, tip, danger).clicked()
                })
                .inner
            };
            if button(ui, "trash", "Supprimer l’étape", true) {
                let l = loc.clone();
                app.edit(move |p| {
                    if let Some(v) = list_mut(p, &l).filter(|v| i < v.len()) {
                        v.remove(i);
                    }
                });
            }
            if i + 1 < len && button(ui, "down", "Descendre", false) {
                step_move(app, loc, i, i + 1);
            }
            if i > 0 && button(ui, "up", "Monter", false) {
                step_move(app, loc, i, i - 1);
            }
            if !open {
                return;
            }
            ui.painter()
                .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, BORDER));
            Frame::new()
                .fill(alpha(CODE_BG, 115))
                .inner_margin(Margin {
                    left: 12,
                    right: 12,
                    top: 4,
                    bottom: 12,
                })
                .corner_radius(CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 10,
                    se: 10,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    form(app, ui, step, loc, i);
                    if let Step::Branch {
                        then, otherwise, ..
                    } = step
                    {
                        for (name, side, list) in
                            [("ALORS", false, then), ("SINON", true, otherwise)]
                        {
                            let r = Frame::new()
                                .inner_margin(Margin {
                                    left: 12,
                                    right: 0,
                                    top: 0,
                                    bottom: 0,
                                })
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(name)
                                            .size(11.0)
                                            .semibold()
                                            .color(hex(0xff8fb8)),
                                    );
                                    steps_box(app, ui, list, &loc.inside_pub(side, i));
                                });
                            let rc = r.response.rect;
                            ui.painter().vline(
                                rc.left() + 3.0,
                                rc.y_range(),
                                Stroke::new(2.0, alpha(hex(0xff66a0), 102)),
                            );
                        }
                    }
                });
        });
}

/// A list of steps: the cards, then a dashed button to add one.
pub fn steps_box(app: &mut App, ui: &mut Ui, list: &[Step], loc: &Loc) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        if list.is_empty() && !loc.nest.is_empty() {
            super::common::empty(ui, "Aucune étape pour l’instant.");
        }
        for (i, step) in list.iter().enumerate() {
            step_card(app, ui, step, loc, i, list.len());
        }
        dashed_add(app, ui, list, loc);
    });
}

/// Gives a step an id of its own (for a copy).
#[allow(dead_code)]
pub fn fresh_id() -> String {
    new_id()
}
