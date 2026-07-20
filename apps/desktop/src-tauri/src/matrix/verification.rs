//! Interactive device verification (SAS / emoji short-auth-string).
//!
//! ⚠️ This is an interactive, stateful handshake between two devices; it is written to the
//! documented `matrix-sdk` API but has **not** been exercised against two live devices from
//! this environment. Validate with a real two-device self-verification before trusting it.
//!
//! Flow: a request is created (or received), driven to the SAS stage, the emojis are
//! surfaced to the UI (`matrix://verification-emojis`), the user confirms they match on both
//! sides (`matrix_confirm_verification`), and completion is signalled
//! (`matrix://verification-done`).

use std::sync::Arc;

use futures_util::StreamExt;
use matrix_sdk::encryption::verification::{
    SasVerification, Verification, VerificationRequest, VerificationRequestState,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::matrix::MatrixState;

pub const VERIFICATION_EMOJIS_EVENT: &str = "matrix://verification-emojis";
pub const VERIFICATION_DONE_EVENT: &str = "matrix://verification-done";
pub const VERIFICATION_REQUEST_EVENT: &str = "matrix://verification-request";

#[derive(Serialize, Clone)]
pub struct EmojiDto {
    pub symbol: String,
    pub description: String,
}

fn emit_emojis(app: &AppHandle, sas: &SasVerification) {
    if let Some(emojis) = sas.emoji() {
        let list: Vec<EmojiDto> = emojis
            .iter()
            .map(|e| EmojiDto {
                symbol: e.symbol.to_string(),
                description: e.description.to_string(),
            })
            .collect();
        let _ = app.emit(VERIFICATION_EMOJIS_EVENT, &list);
    }
}

/// Drive a verification request until it becomes a SAS flow, then hand off to `drive_sas`.
pub async fn drive_request(request: VerificationRequest, app: AppHandle, state: Arc<MatrixState>) {
    let mut stream = Box::pin(request.changes());
    let mut sas: Option<SasVerification> = None;
    while let Some(st) = stream.next().await {
        match st {
            VerificationRequestState::Ready { .. } => {
                if let Ok(Some(s)) = request.start_sas().await {
                    sas = Some(s);
                    break;
                }
            }
            VerificationRequestState::Transitioned { verification } => {
                if let Verification::SasV1(s) = verification {
                    sas = Some(s);
                    break;
                }
            }
            VerificationRequestState::Done | VerificationRequestState::Cancelled(_) => return,
            _ => {}
        }
    }
    if let Some(sas) = sas {
        drive_sas(sas, app, state).await;
    }
}

/// Accept the SAS, publish the emojis to the UI, and watch it to completion.
pub async fn drive_sas(sas: SasVerification, app: AppHandle, state: Arc<MatrixState>) {
    let _ = sas.accept().await;
    *state.sas.write().await = Some(sas.clone());
    emit_emojis(&app, &sas);

    let mut stream = Box::pin(sas.changes());
    while stream.next().await.is_some() {
        emit_emojis(&app, &sas);
        if sas.is_done() {
            let _ = app.emit(VERIFICATION_DONE_EVENT, &true);
            break;
        }
    }
    *state.sas.write().await = None;
}
