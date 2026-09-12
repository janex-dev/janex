// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Explicit local signing keys and certificate trust material.

use crate::{Result, error::invalid};
use janex_format::binary::Limits;
use std::{fs, io::Read, path::Path, time::SystemTime};
use zeroize::Zeroizing;

pub use janex_format::signature::cms::{
    Algorithm as CmsAlgorithm, CmsSigner, RevocationList, SignerCertificate, VerifiedSigner,
};
pub use janex_format::signature::openpgp::{
    Algorithm as OpenPgpAlgorithm, KeyCertificate as OpenPgpCertificate, OpenPgpSigner,
    VerifiedSigner as OpenPgpVerifiedSigner,
};

/// Default CLI limits for an individual key, certificate, or CRL file: 4 MiB and 65,536 values.
pub const MATERIAL_LIMITS: Limits = Limits {
    max_bytes: 4 * 1024 * 1024,
    max_elements: 65_536,
    max_depth: 32,
};

/// Explicit CMS signer pins and offline revocation material; no operating-system trust is implied.
#[derive(Clone, Debug, Default)]
pub struct CmsTrust {
    /// Every listed signer must authenticate the input. An empty list authorizes no CMS signer.
    pub signers: Vec<SignerCertificate>,
    /// Issuer certificates used only to authenticate CRLs for already pinned signers.
    pub issuers: Vec<SignerCertificate>,
    /// Complete direct CRLs checked together with matching embedded CRLs.
    pub revocation_lists: Vec<RevocationList>,
}

/// Records publisher authentication separately from content checksum coverage.
#[derive(Clone, Debug)]
pub enum Authentication {
    /// Execution was explicitly authorized without publisher authentication.
    Unsigned,
    /// Every required CMS certificate passed signature and offline trust checks.
    Cms(Vec<VerifiedSigner>),
    /// The pinned OpenPGP primary key authorized the verified primary or signing subkey.
    OpenPgp(OpenPgpVerifiedSigner),
}

/// Reads one bounded local binary or armored OpenPGP public-key certificate.
pub fn load_openpgp_certificate(path: &Path, limits: Limits) -> Result<OpenPgpCertificate> {
    Ok(OpenPgpCertificate::decode(
        &read_material(path, limits.max_bytes)?,
        limits,
    )?)
}

/// Loads and unlocks one certified OpenPGP signer from a bounded binary or armored secret-key file.
///
/// An absent fingerprint prefers the newest eligible signing subkey, then the primary key.
/// Selection and password-derivation limits are checked before invoking `password`, which runs at
/// most once and only for the selected encrypted key. Input and password buffers are zeroized on
/// drop. The returned signer retains unlocked private material and rechecks policy when signing.
pub fn load_openpgp_signer(
    key: &Path,
    fingerprint: Option<&str>,
    algorithm: Option<OpenPgpAlgorithm>,
    limits: Limits,
    password: impl FnOnce() -> Result<Zeroizing<Vec<u8>>>,
) -> Result<OpenPgpSigner> {
    let bytes = read_material(key, limits.max_bytes)?;
    let signer = OpenPgpSigner::decode(&bytes, fingerprint, algorithm, limits, SystemTime::now())?;
    let password = if signer.is_encrypted() {
        password()?
    } else {
        Zeroizing::new(Vec::new())
    };
    Ok(signer.unlock(&password)?)
}

/// Reads a certificate from a bounded local DER or PEM file.
pub fn load_certificate(path: &Path, limits: Limits) -> Result<SignerCertificate> {
    Ok(SignerCertificate::decode(
        &read_material(path, limits.max_bytes)?,
        limits,
    )?)
}

/// Reads a complete direct CRL from a bounded local DER or PEM file.
pub fn load_revocation_list(path: &Path, limits: Limits) -> Result<RevocationList> {
    Ok(RevocationList::decode(
        &read_material(path, limits.max_bytes)?,
        limits,
    )?)
}

/// Loads a matching CMS certificate and private key, requesting a password only for encrypted keys.
///
/// `password` runs at most once and must obtain the password through the caller's chosen interface.
/// Input buffers and passwords are zeroized on drop. An absent algorithm selects the certificate's
/// preferred supported combination. This function does not publish a package or establish trust.
pub fn load_cms_signer(
    certificate: &Path,
    key: &Path,
    algorithm: Option<CmsAlgorithm>,
    limits: Limits,
    password: impl FnOnce() -> Result<Zeroizing<Vec<u8>>>,
) -> Result<CmsSigner> {
    let certificate = load_certificate(certificate, limits)?;
    let key = read_material(key, limits.max_bytes)?;
    let algorithm = algorithm
        .map(Ok)
        .unwrap_or_else(|| certificate.preferred_algorithm())?;
    let password = if CmsSigner::key_is_encrypted(&key, limits)? {
        Some(password()?)
    } else {
        None
    };
    Ok(CmsSigner::from_pkcs8(
        certificate,
        &key,
        password.as_ref().map(|password| password.as_slice()),
        algorithm,
        limits,
    )?)
}

/// Reads bounded local key or trust bytes into storage zeroized on drop.
pub fn read_material(path: &Path, max_bytes: u64) -> Result<Zeroizing<Vec<u8>>> {
    let limit = max_bytes
        .checked_add(1)
        .ok_or_else(|| invalid("invalid key-material byte limit"))?;
    let mut bytes = Zeroizing::new(Vec::new());
    fs::File::open(path)?.take(limit).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(invalid("key or trust material exceeds the byte limit"));
    }
    Ok(bytes)
}
