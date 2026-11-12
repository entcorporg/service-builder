//! The steps of a flow as cards: a tinted icon, the name of the step and what it does, and a body that opens in place
//! with the fields of the step. A condition holds two more lists of cards, `alors` and `sinon`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib::{self, clone};
use gtk::prelude::*;
use sb_app::scope::Var;
use sb_app::stepform::{self, Kind};
use sb_model::{Assignment, CmpOp, Condition, Expr, Field, FieldType, Step};

use super::assignments::{Update, assignments, expr_editor};
use super::edit::edit;
use super::icons::{self, icon};
use super::inspector::{choice, commit_entry, icon_button};
use super::steps::{Flow, Loc, list_mut, new_id, step_move, vars_at};
use super::ui::Shared;
use super::widgets::say;

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

fn tone(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "log" => ("gray", "#b6bdea"),
        "let" => ("violet", "#c7a0ff"),
        "publish" | "publishLive" => ("orange", "#ffb04a"),
        "callRpc" | "sql" => ("blue", "#7886ff"),
        "branch" => ("pink", "#ff8fb8"),
        "respond" => ("green", "#3ecf8e"),
        "fail" => ("red", "#ff6678"),
        "rateLimit" => ("salmon", "#ff9c6e"),
        "code" => ("cyan", "#7fdcff"),
        "imageSniff" | "objectPut" | "objectGet" | "objectDelete" | "objectPresignGet"
        | "objectPresignPut" => ("teal", "#4ecdc4"),
        _ => ("yellow", "#ffc44d"),
    }
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

fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    l.add_css_class(class);
    l
}

type Commit = Rc<dyn Fn(String)>;

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

/// A value: where it comes from, as a source and what goes with it.
fn expr_widget(
    target: Option<&Field>,
    optional: bool,
    text: &str,
    vars: &[Var],
    commit: Commit,
) -> gtk::Box {
    let apply: Rc<dyn Fn(Option<Expr>)> = Rc::new(move |e| {
        commit(
            e.and_then(|e| serde_json::to_value(e).ok())
                .map(|v| stepform::expr_to_text(&v))
                .unwrap_or_default(),
        )
    });
    expr_editor(target, optional, as_expr(text), vars, apply)
}

const OPS: [(&str, CmpOp); 6] = [
    ("=", CmpOp::Eq),
    ("≠", CmpOp::Ne),
    (">", CmpOp::Gt),
    ("≥", CmpOp::Ge),
    ("<", CmpOp::Lt),
    ("≤", CmpOp::Le),
];

/// `Si` a comparison of two values, or a Rust condition.
fn condition_widget(text: &str, vars: &[Var], commit: Commit) -> gtk::Box {
    let host = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let start = stepform::parse_condition(text)
        .ok()
        .and_then(|v| serde_json::from_value::<Condition>(v).ok())
        .unwrap_or(Condition::Compare {
            left: Expr::Path {
                path: String::new(),
            },
            op: CmpOp::Eq,
            right: Expr::Literal {
                value: serde_json::Value::from(0),
            },
        });
    let state = Rc::new(RefCell::new(start.clone()));
    let push: Rc<dyn Fn()> = {
        let (state, commit) = (state.clone(), commit.clone());
        Rc::new(move || {
            if let Ok(v) = serde_json::to_value(&*state.borrow()) {
                commit(stepform::condition_to_text(&v));
            }
        })
    };
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(&label("Si", "step-field"));
    let kinds = ["comparaison".to_string(), "Rust".to_string()];
    let mode = choice(
        &kinds,
        if matches!(start, Condition::Raw { .. }) {
            "Rust"
        } else {
            "comparaison"
        },
        {
            let (state, push) = (state.clone(), push.clone());
            move |m| {
                let next = if m == "Rust" {
                    Condition::Raw {
                        code: String::new(),
                    }
                } else {
                    Condition::Compare {
                        left: Expr::Path {
                            path: String::new(),
                        },
                        op: CmpOp::Eq,
                        right: Expr::Literal {
                            value: serde_json::Value::from(0),
                        },
                    }
                };
                let same = matches!(
                    (&*state.borrow(), &next),
                    (Condition::Raw { .. }, Condition::Raw { .. })
                        | (Condition::Compare { .. }, Condition::Compare { .. })
                );
                if !same {
                    *state.borrow_mut() = next;
                    push();
                }
            }
        },
    );
    mode.set_hexpand(true);
    head.append(&mode);
    host.append(&head);
    match &start {
        Condition::Compare { left, op, right } => {
            let side = |left_side: bool, e: &Expr| {
                let (state, push) = (state.clone(), push.clone());
                let apply: Rc<dyn Fn(Option<Expr>)> = Rc::new(move |new| {
                    if let (Some(new), Condition::Compare { left, right, .. }) =
                        (new, &mut *state.borrow_mut())
                    {
                        *(if left_side { left } else { right }) = new;
                    }
                    push();
                });
                expr_editor(None, false, Some(e.clone()), vars, apply)
            };
            host.append(&side(true, left));
            let names: Vec<String> = OPS.iter().map(|(n, _)| (*n).to_string()).collect();
            let shown = OPS.iter().find(|(_, o)| o == op).map_or("=", |(n, _)| *n);
            let (st, pu) = (state.clone(), push.clone());
            let op_pick = choice(&names, shown, move |v| {
                if let (Some((_, o)), Condition::Compare { op, .. }) =
                    (OPS.iter().find(|(n, _)| *n == v), &mut *st.borrow_mut())
                {
                    *op = *o;
                }
                pu();
            });
            op_pick.set_halign(gtk::Align::Start);
            host.append(&op_pick);
            host.append(&side(false, right));
        }
        Condition::Raw { code } => {
            let (st, pu) = (state.clone(), push.clone());
            host.append(&commit_entry(
                code,
                true,
                "condition Rust, par exemple input.amount > 0",
                true,
                move |v| {
                    *st.borrow_mut() = Condition::Raw { code: v };
                    pu();
                },
            ));
        }
    }
    host
}

/// The message an assignment list fills, when it is known: the payload of a topic, the request of an RPC, the response.
fn assignment_target(ui: &Shared, step: &Step, key: &str, loc: &Loc) -> Option<String> {
    let v = serde_json::to_value(step).ok()?;
    let s = ui.state.borrow();
    let p = &s.project;
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
fn form(ui: &Shared, step: &Step, loc: &Loc, index: usize) -> gtk::Box {
    let host = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let all = stepform::to_texts(step);
    let texts: Rc<RefCell<Vec<String>>> =
        Rc::new(RefCell::new(all.iter().map(|(_, t)| t.clone()).collect()));
    let messages: Vec<String> = ui
        .state
        .borrow()
        .project
        .messages
        .iter()
        .map(|m| m.name.clone())
        .collect();
    let vars = vars_at(ui, loc, index);
    let target = let_target(step);
    let is_fail = matches!(step, Step::Fail { .. });
    let fail_status: String = all
        .iter()
        .find(|(s, _)| s.key == "status")
        .map(|(_, t)| t.clone())
        .unwrap_or_default();
    for (i, (spec, text)) in all.into_iter().enumerate() {
        // The `Retry-After` header only means something for 429 and 503.
        if is_fail && spec.key == "retryAfter" && !matches!(fail_status.as_str(), "429" | "503") {
            continue;
        }
        let commit: Commit = Rc::new({
            let (ui, step, loc, texts) = (ui.clone(), step.clone(), loc.clone(), texts.clone());
            move |v| {
                texts.borrow_mut()[i] = v;
                match stepform::from_texts(&step, &texts.borrow()) {
                    Ok(new) => {
                        let loc = loc.clone();
                        edit(&ui, move |p| {
                            if let Some(slot) = list_mut(p, &loc).and_then(|l| l.get_mut(index)) {
                                *slot = new;
                            }
                        })
                    }
                    Err(e) => say(&ui, &e),
                }
            }
        });
        let name = label(&field_name(&spec.key), "step-field");
        // A structure is edited with widgets (and a schema picked in a list), never typed as JSON. The assignments of a
        // schema keep their own editor.
        let by_schema =
            matches!(spec.kind, Kind::Assignments) && assignment_target(ui, step, &spec.key, loc).is_some();
        if !by_schema
            && let Some(editor) = super::stepjson::structured(
                ui,
                step,
                loc.service,
                &spec,
                &text,
                &vars,
                commit.clone(),
            )
        {
            host.append(&name);
            host.append(&editor);
            if !spec.help.is_empty() {
                host.append(&label(&spec.help, "row-hint"));
            }
            continue;
        }
        let control: gtk::Widget = match &spec.kind {
            Kind::Number if is_fail && spec.key == "status" => {
                let words: Vec<String> = FAIL_STATUSES
                    .iter()
                    .map(|(c, w)| format!("{c} · {w}"))
                    .collect();
                let now = FAIL_STATUSES
                    .iter()
                    .find(|(c, _)| c.to_string() == text)
                    .map_or(text.clone(), |(c, w)| format!("{c} · {w}"));
                choice(&words, &now, move |v| {
                    commit(v.split(' ').next().unwrap_or("400").to_string())
                })
                .upcast()
            }
            Kind::Bool => {
                let c = gtk::CheckButton::with_label(&field_name(&spec.key));
                c.set_active(text == "true");
                c.connect_toggled(move |c| commit(c.is_active().to_string()));
                host.append(&c);
                if !spec.help.is_empty() {
                    host.append(&label(&spec.help, "row-hint"));
                }
                continue;
            }
            Kind::Choice(words) => choice(words, &text, move |v| commit(v)).upcast(),
            Kind::Type => {
                let mut options: Vec<String> =
                    stepform::SCALARS.iter().map(|s| (*s).to_string()).collect();
                options.extend(messages.iter().cloned());
                choice(&options, &text, move |v| commit(v)).upcast()
            }
            Kind::Expr => expr_widget(
                target.as_ref().filter(|_| spec.key == "value"),
                spec.optional,
                &text,
                &vars,
                commit,
            )
            .upcast(),
            Kind::Condition => {
                host.append(&condition_widget(&text, &vars, commit));
                if !spec.help.is_empty() {
                    host.append(&label(&spec.help, "row-hint"));
                }
                continue;
            }
            Kind::Assignments => match assignment_target(ui, step, &spec.key, loc) {
                Some(message) => {
                    let list: Vec<Assignment> = stepform::parse_assignments(&text)
                        .ok()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    let update: Update = {
                        let (list, commit) = (list.clone(), commit.clone());
                        Rc::new(move |f| {
                            let mut next = list.clone();
                            f(&mut next);
                            commit(stepform::assignments_to_text(
                                &serde_json::to_value(&next).unwrap_or_default(),
                            ));
                        })
                    };
                    assignments(ui, &message, &list, &vars, update).upcast()
                }
                // Handled above (`stepjson`): a list with no schema is a list of names and values.
                None => label("", "row-hint").upcast(),
            },
            // Never typed by hand: a field of this shape needs its own widgets (`stepjson`) or a schema.
            Kind::Json => label(
                "Ce champ se règle par un schéma ou dans le fichier du projet, pas en JSON saisi à la main.",
                "row-hint",
            )
            .upcast(),
            Kind::Number | Kind::Text | Kind::Words => commit_entry(
                &text,
                !matches!(spec.kind, Kind::Text),
                "",
                true,
                move |v| commit(v),
            )
            .upcast(),
        };
        host.append(&name);
        host.append(&control);
        if !spec.help.is_empty() {
            host.append(&label(&spec.help, "row-hint"));
        }
    }
    host
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
        "publishLive" => "Événement temps réel (SSE), envoyé après le commit, à un compte ou à une permission",
        "callRpc" => "Appeler un autre service en gRPC",
        "branch" => "Si … alors … sinon …",
        "respond" => "Renvoyer la réponse du RPC",
        "signJwt" => "Access token Ed25519 (EdDSA) : vos claims, plus émetteur, émission et expiration",
        "verifyJwt" => "Signature, émetteur et expiration ; les claims lus comme un schéma. Sinon 401",
        "jwks" => "Le JWKS : les clés publiques qui permettent de vérifier un jeton, jamais la graine",
        "totpGenerate" => "Secret TOTP chiffré à stocker, URI otpauth:// pour le QR code",
        "totpVerify" => "Fenêtre ±30 s, un code ne sert qu’une fois (anti-rejeu). Sinon 401",
        "recoveryCodes" => "XXXXX-XXXXX, à montrer une fois ; seules leurs empreintes sont stockées",
        "recoveryCodeUse" => "Valable une seule fois, avec ou sans tiret. Sinon 401",
        "passkeyRegisterStart" => "Un défi pour le navigateur (WebAuthn) ; le compte est déjà authentifié",
        "passkeyRegisterFinish" => "Vérifie la réponse de l’appareil, une seule fois, et stocke la passkey. Sinon 400",
        "passkeyLoginStart" => "Un défi pour les passkeys du compte, ou sans compte : l’appareil propose les siennes. Sans passkey : 401",
        "passkeyLoginFinish" => "Vérifie la réponse et donne le compte. Sinon 401",
        "imageSniff" => "PNG ou JPEG d’après les premiers octets, jamais le Content-Type du client ; pas de SVG. Sinon 400",
        "objectPut" => "Dans un bucket privé (S3 : Garage, MinIO ou AWS). L’objet d’abord, la base ensuite",
        "objectGet" => "En base64 ; rien s’il n’existe pas. Stockage injoignable : 503",
        "objectPresignGet" => "Le front lit l’objet directement (stockage ou CDN) : rien ne transite par l’API. Durée limitée",
        "objectPresignPut" => "Le front envoie le fichier directement au stockage : type et taille sont signés. Durée limitée",
        "objectDelete" => "La base d’abord, l’objet ensuite. Un objet absent n’est pas une erreur",
        "encrypt" => "AES-256-GCM, pour ce qui est stocké (un secret TOTP, un jeton)",
        "decrypt" => "Le texte qu’un bloc « Chiffrer » a scellé",
        "verifyHmac" => "Webhook signé (HMAC-SHA256 du corps brut), sans session : toujours la première étape",
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
fn allowed(ui: &Shared, loc: &Loc) -> Vec<&'static str> {
    let has_response = matches!(loc.flow, Flow::Route(_) | Flow::Rpc(_));
    let http = matches!(loc.flow, Flow::Route(_));
    let (has_realtime, has_signature) = {
        let s = ui.state.borrow();
        let mut p = s.project.clone();
        let has_signature = list_mut(&mut p, loc)
            .is_some_and(|l| l.iter().any(|x| matches!(x, Step::VerifyHmac { .. })));
        (
            s.project
                .services
                .iter()
                .any(|x| x.kind == sb_model::ServiceKind::Realtime),
            has_signature,
        )
    };
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
fn add_kind(ui: &Shared, loc: &Loc, kind: &str) {
    let id = new_id();
    match stepform::blank(kind, &id) {
        Ok(step) => {
            ui.canvas.state.borrow_mut().open.insert(format!("step/{id}"));
            let (loc, first) = (loc.clone(), kind == "verifyHmac");
            edit(ui, move |p| {
                if let Some(list) = list_mut(p, &loc) {
                    if first {
                        list.insert(0, step);
                    } else {
                        list.push(step);
                    }
                }
            });
        }
        Err(e) => say(ui, &e),
    }
}

/// The dashed button, and under it, once opened, every block in a grid of two columns (no dialog).
fn dashed_add(ui: &Shared, loc: &Loc) -> gtk::Box {
    let host = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let add = gtk::Button::new();
    add.add_css_class("add-step-btn");
    let c = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    c.set_halign(gtk::Align::Center);
    c.append(&icon("plus", 14, icons::MUTED));
    c.append(&gtk::Label::new(Some("Ajouter une étape")));
    add.set_child(Some(&c));
    host.append(&add);
    let grid_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    host.append(&grid_slot);
    let (ui, loc) = (ui.clone(), loc.clone());
    add.connect_clicked(move |_| {
        if grid_slot.first_child().is_some() {
            while let Some(old) = grid_slot.first_child() {
                grid_slot.remove(&old);
            }
            return;
        }
        let grid = gtk::Grid::new();
        grid.add_css_class("widget-grid");
        grid.set_column_homogeneous(true);
        grid.set_column_spacing(6);
        grid.set_row_spacing(6);
        for (n, kind) in allowed(&ui, &loc).into_iter().enumerate() {
            grid.attach(&widget(&ui, &loc, kind), (n % 2) as i32, (n / 2) as i32, 1, 1);
        }
        grid_slot.append(&grid);
    });
    // `SB_OPEN_ADD`, with `SB_SCREENSHOT`: the grid of blocks open.
    if std::env::var_os("SB_OPEN_ADD").is_some() {
        add.emit_clicked();
    }
    host
}

/// One block of the grid: its icon, its name and what it does.
fn widget(ui: &Shared, loc: &Loc, kind: &'static str) -> gtk::Button {
    let (_, color) = tone(kind);
    let b = gtk::Button::new();
    b.add_css_class("widget");
    let cell = gtk::Grid::new();
    cell.set_column_spacing(8);
    let ic = super::icons::centered(kind, 18, color, 24);
    ic.set_valign(gtk::Align::Center);
    cell.attach(&ic, 0, 0, 1, 2);
    let name = label(title(kind), "widget-name");
    name.set_hexpand(true);
    name.set_max_width_chars(1);
    cell.attach(&name, 1, 0, 1, 1);
    let what = label(hint(kind), "widget-hint");
    what.set_max_width_chars(1);
    cell.attach(&what, 1, 1, 1, 1);
    b.set_child(Some(&cell));
    let (ui, loc) = (ui.clone(), loc.clone());
    b.connect_clicked(move |_| add_kind(&ui, &loc, kind));
    b
}

fn card(ui: &Shared, step: &Step, loc: &Loc, i: usize, len: usize) -> gtk::Box {
    let kind = serde_json::to_value(step)
        .ok()
        .and_then(|v| v["type"].as_str().map(String::from))
        .unwrap_or_default();
    let (group, color) = tone(&kind);
    let id = format!("step/{}", step.id());
    let open = ui.canvas.state.borrow().open.contains(&id) || std::env::var_os("SB_OPEN_ALL").is_some();

    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("step-card");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let toggle = gtk::Button::new();
    toggle.add_css_class("step-open");
    toggle.set_hexpand(true);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let chevron = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    chevron.append(&icon(if open { "down" } else { "right" }, 14, icons::FAINT));
    line.append(&chevron);
    let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    badge.add_css_class("step-icon");
    badge.add_css_class(&format!("si-{group}"));
    badge.set_valign(gtk::Align::Center);
    badge.append(&super::icons::centered(&kind, 14, color, 24));
    line.append(&badge);
    let t = gtk::Label::new(Some(title(&kind)));
    t.add_css_class("step-kind");
    line.append(&t);
    let s = gtk::Label::builder()
        .label(summary(step, &kind))
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    s.add_css_class("step-summary");
    line.append(&s);
    toggle.set_child(Some(&line));
    head.append(&toggle);
    if i > 0 {
        let (u, l) = (ui.clone(), loc.clone());
        head.append(&icon_button("up", "Monter", false, move || {
            step_move(&u, &l, i, i - 1)
        }));
    }
    if i + 1 < len {
        let (u, l) = (ui.clone(), loc.clone());
        head.append(&icon_button("down", "Descendre", false, move || {
            step_move(&u, &l, i, i + 1)
        }));
    }
    let (u, l) = (ui.clone(), loc.clone());
    head.append(&icon_button(
        "trash",
        "Supprimer l’étape",
        true,
        move || {
            let l = l.clone();
            edit(&u, move |p| {
                if let Some(v) = list_mut(p, &l).filter(|v| i < v.len()) {
                    v.remove(i);
                }
            })
        },
    ));
    outer.append(&head);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.add_css_class("step-body");
    body.set_visible(open);
    body.append(&form(ui, step, loc, i));
    if let Step::Branch {
        then, otherwise, ..
    } = step
    {
        for (name, side, list) in [("ALORS", false, then), ("SINON", true, otherwise)] {
            let branch = gtk::Box::new(gtk::Orientation::Vertical, 6);
            branch.add_css_class("branch");
            branch.append(&label(name, "branch-title"));
            branch.append(&steps_box(ui, list, &loc.inside_pub(side, i)));
            body.append(&branch);
        }
    }
    outer.append(&body);

    toggle.connect_clicked(clone!(
        #[strong]
        ui,
        #[weak]
        body,
        #[weak]
        chevron,
        move |_| {
            let now = !body.is_visible();
            body.set_visible(now);
            if let Some(old) = chevron.first_child() {
                chevron.remove(&old);
            }
            chevron.append(&icon(if now { "down" } else { "right" }, 14, icons::FAINT));
            let mut st = ui.canvas.state.borrow_mut();
            if now {
                st.open.insert(id.clone());
            } else {
                st.open.remove(&id);
            }
        }
    ));
    outer
}

/// A list of steps: the cards, then a dashed button to add one.
pub fn steps_box(ui: &Shared, list: &[Step], loc: &Loc) -> gtk::Box {
    let host = gtk::Box::new(gtk::Orientation::Vertical, 8);
    if list.is_empty() && !loc.nest.is_empty() {
        host.append(&label("Aucune étape pour l’instant.", "empty"));
    }
    for (i, step) in list.iter().enumerate() {
        host.append(&card(ui, step, loc, i, list.len()));
    }
    host.append(&dashed_add(ui, loc));
    host
}

/// Gives a step an id of its own (for a copy).
#[allow(dead_code)]
pub fn fresh_id() -> String {
    new_id()
}
