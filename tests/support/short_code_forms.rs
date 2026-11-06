//! Everything a short code could be seen as, were anything to let it out, for
//! the tests that look for it where it must never be (ADR-0065). A keyed value
//! is not among them, having nothing here to be computed from: what catches one
//! is that each message on the wire is exactly as long as it should be, and
//! differs between two codes only where the exchange says it must.

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ring::digest;
use suru::protocol::ShortCode;

/// What `bytes` reads as under each way bytes come to be written out, itself
/// included.
fn written_out(bytes: &[u8]) -> Vec<Vec<u8>> {
    vec![
        bytes.to_vec(),
        URL_SAFE_NO_PAD.encode(bytes).into_bytes(),
        STANDARD.encode(bytes).into_bytes(),
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
            .into_bytes(),
        format!("{bytes:?}").into_bytes(),
    ]
}

/// Each form of `code`: as it is written with and without its separator, in
/// either case, and each of those and its hashes as bytes come to be written
/// out, as the `Debug` of a byte slice among them.
pub fn forms_of(code: &ShortCode) -> Vec<Vec<u8>> {
    let grouped = code.grouped();
    let canonical = grouped.replace('-', "");
    let mut forms = Vec::new();
    for written in [
        canonical.clone(),
        grouped.clone(),
        canonical.to_lowercase(),
        grouped.to_lowercase(),
    ] {
        forms.extend(written_out(written.as_bytes()));
    }
    for hashed in [canonical.as_bytes(), grouped.as_bytes()] {
        for algorithm in [&digest::SHA256, &digest::SHA512] {
            forms.extend(written_out(digest::digest(algorithm, hashed).as_ref()));
        }
        forms.extend(written_out(blake3::hash(hashed).as_bytes()));
    }
    forms
}

/// The first form of `code` that `seen` holds, if any does.
pub fn form_held<'a>(seen: &[u8], forms: &'a [Vec<u8>]) -> Option<&'a [u8]> {
    forms
        .iter()
        .find(|form| seen.windows(form.len()).any(|at| at == form.as_slice()))
        .map(Vec::as_slice)
}

/// A code that is not `code`, for a guess that is wrong.
pub fn another_code(code: &ShortCode) -> ShortCode {
    let other = if code.grouped() == "2222-2222" {
        "3333-3333"
    } else {
        "2222-2222"
    };
    ShortCode::parse(other).unwrap()
}
