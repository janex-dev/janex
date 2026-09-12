// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Offline revocation checks for complete, direct X.509 CRLs.

use super::{
    Algorithm, ECDSA_SHA256, ECDSA_SHA384, RSA_SHA256, RSA_SHA512, SignerCertificate, decode_der,
    der_error, invalid, pem_or_der, public_key, same_algorithm, trust, unsupported, verify_bytes,
};
use crate::{Result, binary::Limits};
use ::cms::{cert::CertificateChoices, revocation::RevocationInfoChoice, signed_data::SignedData};
use der::{Decode, Encode};
use std::{collections::BTreeSet, time::SystemTime};
use x509_cert::{
    Certificate, Version,
    crl::CertificateList,
    ext::{
        Extensions,
        pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage, crl::CrlReason},
    },
    spki::AlgorithmIdentifierOwned,
};

/// A complete direct X.509 v2 CRL, whose authenticity is checked against the pinned signer's issuer.
#[derive(Clone, Debug)]
pub struct RevocationList {
    /// Canonical decoded CRL.
    list: CertificateList,
}

impl RevocationList {
    /// Reads one DER or PEM X509 CRL without treating its issuer or contents as trusted.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        let bytes = pem_or_der(bytes, "X509 CRL")?;
        Ok(Self {
            list: decode_der(&bytes, limits)?,
        })
    }
}

/// Verifies every matching CRL and returns the number of authenticated lists checked.
pub(super) fn check(
    data: &SignedData,
    certificate: &SignerCertificate,
    issuers: &[SignerCertificate],
    supplied: &[RevocationList],
    now: SystemTime,
) -> Result<usize> {
    let mut candidates = vec![&certificate.certificate];
    candidates.extend(issuers.iter().map(|issuer| &issuer.certificate));
    if let Some(certificates) = &data.certificates {
        candidates.extend(certificates.0.iter().filter_map(|choice| match choice {
            CertificateChoices::Certificate(certificate) => Some(certificate),
            _ => None,
        }));
    }
    let mut lists: Vec<_> = supplied.iter().map(|crl| &crl.list).collect();
    if let Some(crls) = &data.crls {
        lists.extend(crls.0.iter().filter_map(|choice| match choice {
            RevocationInfoChoice::Crl(crl) => Some(crl),
            _ => None,
        }));
    }
    let mut checked = 0;
    for list in lists {
        if list.tbs_cert_list.issuer != certificate.certificate.tbs_certificate.issuer {
            continue;
        }
        let mut authenticated = false;
        let mut failure = None;
        for issuer in &candidates {
            if issuer.tbs_certificate.subject != list.tbs_cert_list.issuer {
                continue;
            }
            match verify_issuer(&certificate.certificate, issuer, list, now) {
                Ok(()) => {
                    authenticated = true;
                    break;
                }
                Err(error) => failure = Some(error),
            }
        }
        if !authenticated {
            return Err(failure
                .unwrap_or_else(|| trust("cannot authenticate a matching CMS revocation list")));
        }
        validate_list(list, &certificate.certificate, now)?;
        checked += 1;
    }
    Ok(checked)
}

/// Authenticates an issuer key through the already pinned certificate and verifies its CRL.
fn verify_issuer(
    certificate: &Certificate,
    issuer: &Certificate,
    list: &CertificateList,
    now: SystemTime,
) -> Result<()> {
    let key = public_key(issuer)?;
    if !same_algorithm(
        &certificate.signature_algorithm,
        &certificate.tbs_certificate.signature,
    ) {
        return Err(invalid("certificate signature algorithms disagree"));
    }
    verify_bytes(
        &key,
        algorithm(&certificate.signature_algorithm)?,
        &certificate.tbs_certificate.to_der().map_err(der_error)?,
        certificate
            .signature
            .as_bytes()
            .ok_or_else(|| invalid("certificate signature is not byte-aligned"))?,
    )?;
    let tbs = &issuer.tbs_certificate;
    if now < tbs.validity.not_before.to_system_time()
        || now > tbs.validity.not_after.to_system_time()
    {
        return Err(trust(
            "CRL issuer certificate is not valid at the verification time",
        ));
    }
    let mut seen = BTreeSet::new();
    for extension in tbs.extensions.as_deref().unwrap_or_default() {
        if !seen.insert(extension.extn_id) {
            return Err(invalid("duplicate CRL issuer certificate extension"));
        }
        if extension.critical
            && !matches!(
                extension.extn_id.to_string().as_str(),
                "2.5.29.15" | "2.5.29.19" | "2.5.29.37"
            )
        {
            return Err(unsupported(
                "unsupported critical CRL issuer certificate extension",
            ));
        }
    }
    if let Some((_, usage)) = tbs.get::<KeyUsage>().map_err(der_error)?
        && !usage.crl_sign()
    {
        return Err(trust("issuer certificate does not permit CRL signing"));
    }
    tbs.get::<BasicConstraints>().map_err(der_error)?;
    tbs.get::<ExtendedKeyUsage>().map_err(der_error)?;
    if !same_algorithm(&list.signature_algorithm, &list.tbs_cert_list.signature) {
        return Err(invalid("CRL signature algorithms disagree"));
    }
    verify_bytes(
        &key,
        algorithm(&list.signature_algorithm)?,
        &list.tbs_cert_list.to_der().map_err(der_error)?,
        list.signature
            .as_bytes()
            .ok_or_else(|| invalid("CRL signature is not byte-aligned"))?,
    )
}

/// Maps supported certificate and CRL signature identifiers to their digest combination.
fn algorithm(identifier: &AlgorithmIdentifierOwned) -> Result<Algorithm> {
    match identifier.oid {
        RSA_SHA256 if super::null_or_absent(&identifier.parameters) => Ok(Algorithm::RsaSha256),
        RSA_SHA512 if super::null_or_absent(&identifier.parameters) => Ok(Algorithm::RsaSha512),
        ECDSA_SHA256 if identifier.parameters.is_none() => Ok(Algorithm::EcdsaP256Sha256),
        ECDSA_SHA384 if identifier.parameters.is_none() => Ok(Algorithm::EcdsaP384Sha384),
        _ => Err(unsupported(
            "unsupported certificate or CRL signature algorithm",
        )),
    }
}

/// Checks CRL time, scope, and revocation entries after issuer authentication succeeds.
fn validate_list(list: &CertificateList, certificate: &Certificate, now: SystemTime) -> Result<()> {
    let tbs = &list.tbs_cert_list;
    if tbs.version != Version::V2 {
        return Err(unsupported("only X.509 v2 CRLs are supported"));
    }
    let next = tbs
        .next_update
        .ok_or_else(|| invalid("CRL omits nextUpdate"))?;
    if now < tbs.this_update.to_system_time() || now > next.to_system_time() {
        return Err(trust("CMS revocation list is not current"));
    }
    extensions(tbs.crl_extensions.as_ref())?;
    let mut serials = BTreeSet::new();
    for entry in tbs.revoked_certificates.as_deref().unwrap_or_default() {
        if !serials.insert(entry.serial_number.as_bytes()) {
            return Err(invalid("duplicate CRL certificate serial number"));
        }
        extensions(entry.crl_entry_extensions.as_ref())?;
        if entry.serial_number == certificate.tbs_certificate.serial_number
            && entry.revocation_date.to_system_time() <= now
        {
            return Err(trust("CMS signer certificate is revoked"));
        }
    }
    Ok(())
}

/// Rejects unsupported scoped CRLs and critical extensions instead of treating them as complete lists.
fn extensions(extensions: Option<&Extensions>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for extension in extensions.into_iter().flatten() {
        if !seen.insert(extension.extn_id) {
            return Err(invalid("duplicate CRL extension"));
        }
        let oid = extension.extn_id.to_string();
        if extension.critical || matches!(oid.as_str(), "2.5.29.27" | "2.5.29.28" | "2.5.29.29") {
            return Err(unsupported(
                "critical, delta, partitioned, or indirect CRL extensions are unsupported",
            ));
        }
        if oid == "2.5.29.21"
            && CrlReason::from_der(extension.extn_value.as_bytes()).map_err(der_error)?
                == CrlReason::RemoveFromCRL
        {
            return Err(unsupported("removeFromCRL requires delta CRL processing"));
        }
    }
    Ok(())
}
