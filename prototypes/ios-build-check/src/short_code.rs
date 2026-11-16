//! The exchange a short code is fed to, after `src/serving/short_invite.rs`: SPAKE2 over Ed25519, each share held
//! to one point written the one way. The first cut of the phone redeems no short code; this is here to say what
//! it would cost to link.

use curve25519_dalek::edwards::CompressedEdwardsY;
use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::Failure;

/// Both sides of the exchange by `code`, and whether they came to one key.
pub fn agree(code: &str) -> Result<bool, Failure> {
    let (password, redeemer, issuer) = (
        Password::new(code.as_bytes()),
        Identity::new(b"redeemer"),
        Identity::new(b"issuer"),
    );
    let (ours, our_share) = Spake2::<Ed25519Group>::start_a(&password, &redeemer, &issuer);
    let (theirs, their_share) = Spake2::<Ed25519Group>::start_b(&password, &redeemer, &issuer);
    for share in [&our_share, &their_share] {
        let point = CompressedEdwardsY::from_slice(&share[1..])?;
        if point.decompress().is_none() {
            return Err("a share is not a point".into());
        }
    }
    let our_key = ours
        .finish(&their_share)
        .map_err(|error| format!("{error:?}"))?;
    let their_key = theirs
        .finish(&our_share)
        .map_err(|error| format!("{error:?}"))?;
    Ok(our_key == their_key)
}

/// `short-code <code>`
pub fn exchange(arguments: &[String]) -> Result<(), Failure> {
    let code = arguments.first().map_or("abcd2345", String::as_str);
    println!("short code: both sides agree: {}", agree(code)?);
    Ok(())
}
