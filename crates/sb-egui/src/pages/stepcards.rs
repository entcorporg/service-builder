//! The steps of a flow as cards: a tinted icon, the name of the step and what it does, and a body that opens in place
//! with the fields of the step. A condition holds two more lists of cards, `alors` and `sinon`.
//! PORT TARGET: `sb-gtk/src/uis/stepcards.rs` (owner: steps agent).

use eframe::egui::Ui;
use sb_model::Step;

use super::steps::Loc;
use crate::app::App;

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

/// A list of steps: the cards, then a dashed button to add one.
pub fn steps_box(app: &mut App, ui: &mut Ui, list: &[Step], loc: &Loc) {
    let _ = (app, list, loc);
    super::common::empty(ui, "À porter : stepcards.rs (steps_box)");
}

/// Gives a step an id of its own (for a copy).
#[allow(dead_code)]
pub fn fresh_id() -> String {
    super::steps::new_id()
}
