//! Engine Contract v1 envelope (contract/engine-v1/envelope.schema.json).
//!
//! stdout carries exactly one envelope JSON payload; human text goes to stderr.
//! Field order mirrors the Python engine's insertion order for readability.

use serde::Serialize;
use serde_json::Value;

pub const CONTRACT_ID: &str = "agent-manager/engine/v1";

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub element: Option<String>,
    pub message: String,
}

pub fn diagnostic(code: &str, element: Option<String>, message: String) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        element,
        message,
    }
}

#[derive(Serialize, Debug)]
pub struct Mode {
    pub offline: bool,
}

#[derive(Serialize, Debug)]
pub struct Lock {
    pub held: bool,
    pub pid: Option<i64>,
}

#[derive(Serialize, Debug)]
pub struct Envelope {
    pub contract: String,
    pub run_id: String,
    pub command: String,
    pub mode: Mode,
    pub lock: Lock,
    pub log_file: Option<String>,
    pub ok: bool,
    pub data: Value,
    pub warnings: Vec<Diagnostic>,
    pub errors: Vec<Diagnostic>,
}

impl Envelope {
    pub fn new(
        command: String,
        mode: Mode,
        lock: Lock,
        run_id: String,
        log_file: Option<String>,
        data: Value,
        warnings: Vec<Diagnostic>,
        errors: Vec<Diagnostic>,
    ) -> Self {
        Envelope {
            contract: CONTRACT_ID.to_string(),
            ok: errors.is_empty(),
            run_id,
            command,
            mode,
            lock,
            log_file,
            data,
            warnings,
            errors,
        }
    }

    /// Serialize like Python's json.dump(indent=2, ensure_ascii=False) + newline.
    pub fn emit(&self) {
        let rendered = serde_json::to_string_pretty(self).expect("envelope serializes");
        println!("{}", rendered);
    }
}
