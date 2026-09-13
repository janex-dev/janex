// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Canonical ECMA-427 Package URLs with the registered type rules used by the format reader.

use crate::{Result, error::invalid};
use packageurl::PackageUrl as RawPackageUrl;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    fmt,
};

/// Immutable decoded components retaining their original canonical representation.
#[derive(Clone, Debug)]
pub struct PackageUrl {
    /// Validated type-specific component values.
    value: RawPackageUrl<'static>,
    /// Canonical ECMA-427 spelling, preserved independently of the component library's formatter.
    encoded: String,
}

impl PackageUrl {
    /// Returns the lowercase ASCII type.
    pub fn ty(&self) -> &str {
        self.value.ty()
    }
    /// Returns the decoded namespace, or `None` when omitted.
    pub fn namespace(&self) -> Option<&str> {
        self.value.namespace()
    }
    /// Returns the decoded nonempty name.
    pub fn name(&self) -> &str {
        self.value.name()
    }
    /// Returns the decoded version, or `None` when omitted.
    pub fn version(&self) -> Option<&str> {
        self.value.version()
    }
    /// Returns the decoded relative subpath, or `None` when omitted.
    pub fn subpath(&self) -> Option<&str> {
        self.value.subpath()
    }
    /// Borrows decoded qualifiers; iteration order is unspecified.
    pub fn qualifiers(&self) -> &HashMap<Cow<'static, str>, Cow<'static, str>> {
        self.value.qualifiers()
    }
}

impl fmt::Display for PackageUrl {
    /// Writes the retained canonical spelling without re-encoding decoded components.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.encoded)
    }
}

/// Parses a canonical Package URL without acquiring its named package.
///
/// Components use uppercase UTF-8 percent escapes; unreserved characters and colons remain
/// literal. Qualifiers are unique, nonempty, and sorted by key. Unknown types retain the generic
/// component rules. Empty versions, qualifier lists, and subpaths must omit their separators.
pub fn parse(uri: &str) -> Result<PackageUrl> {
    let rest = uri
        .strip_prefix("pkg:")
        .ok_or_else(|| invalid("invalid Package URL scheme"))?;
    let (rest, subpath) = match rest.split_once('#') {
        Some((rest, value)) => (rest, Some(segments(value, true)?)),
        None => (rest, None),
    };
    let (rest, qualifiers) = match rest.split_once('?') {
        Some((rest, value)) => {
            let mut qualifiers = BTreeMap::new();
            let mut previous: Option<&str> = None;
            for pair in value.split('&') {
                let (key, value) = pair
                    .split_once('=')
                    .ok_or_else(|| invalid("invalid Package URL qualifier"))?;
                token(key, true)?;
                if previous.is_some_and(|old| old >= key) {
                    return Err(invalid("Package URL qualifiers must be unique and sorted"));
                }
                previous = Some(key);
                qualifiers.insert(key.to_owned(), component(value)?);
            }
            (rest, qualifiers)
        }
        None => (rest, BTreeMap::new()),
    };
    let (rest, version) = match rest.split_once('@') {
        Some((rest, value)) => (rest, Some(component(value)?)),
        None => (rest, None),
    };
    let (ty, path) = rest
        .split_once('/')
        .ok_or_else(|| invalid("missing Package URL type"))?;
    token(ty, false)?;
    let (namespace, name) = match path.rsplit_once('/') {
        Some((namespace, name)) => (Some(segments(namespace, false)?), component(name)?),
        None => (None, component(path)?),
    };
    let lower_name = matches!(
        ty,
        "bitbucket" | "composer" | "deb" | "github" | "hex" | "npm" | "pypi"
    ) || ty == "mlflow"
        && qualifiers
            .get("repository_url")
            .is_some_and(|value| value.contains("databricks"));
    let lower_namespace = matches!(
        ty,
        "apk" | "bitbucket" | "composer" | "deb" | "github" | "golang" | "hex" | "qpkg" | "rpm"
    );
    if lower_name && name.to_lowercase() != name
        || lower_namespace
            && namespace
                .as_ref()
                .is_some_and(|value| value.to_lowercase() != *value)
        || ty == "huggingface"
            && version
                .as_ref()
                .is_some_and(|value| value.to_lowercase() != *value)
    {
        return Err(invalid("noncanonical Package URL case"));
    }
    let mut purl = RawPackageUrl::new(ty.to_owned(), name.clone())
        .map_err(|_| invalid("invalid Package URL name"))?;
    if purl.name() != name {
        return Err(invalid("noncanonical Package URL name"));
    }
    if let Some(namespace) = namespace {
        purl.with_namespace(namespace)
            .map_err(|_| invalid("invalid Package URL namespace"))?;
    }
    if let Some(version) = version {
        purl.with_version(version)
            .map_err(|_| invalid("invalid Package URL version"))?;
    }
    if let Some(subpath) = subpath {
        purl.with_subpath(subpath)
            .map_err(|_| invalid("invalid Package URL subpath"))?;
    }
    for (key, value) in qualifiers {
        purl.add_qualifier(key, value)
            .map_err(|_| invalid("invalid Package URL qualifier"))?;
    }
    purl.validate()
        .map_err(|_| invalid("Package URL violates its type definition"))?;
    Ok(PackageUrl {
        value: purl,
        encoded: uri.to_owned(),
    })
}

/// Validates a lowercase ASCII type or qualifier key.
fn token(value: &str, qualifier: bool) -> Result<()> {
    if !value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'-')
                || qualifier && byte == b'_'
        })
    {
        return Err(invalid("invalid Package URL type or qualifier key"));
    }
    Ok(())
}

/// Checks the only characters permitted literally inside a canonical component.
fn literal(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'~' | b':')
}

/// Decodes one nonempty component while enforcing exact percent-escape spelling.
fn component(value: &str) -> Result<String> {
    if value.is_empty() {
        return Err(invalid("empty Package URL component"));
    }
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input
                .next()
                .and_then(hex)
                .ok_or_else(|| invalid("invalid Package URL escape"))?;
            let low = input
                .next()
                .and_then(hex)
                .ok_or_else(|| invalid("invalid Package URL escape"))?;
            let decoded = high * 16 + low;
            if literal(decoded) {
                return Err(invalid("unnecessary Package URL escape"));
            }
            bytes.push(decoded);
        } else if literal(byte) {
            bytes.push(byte);
        } else {
            return Err(invalid("unescaped Package URL component character"));
        }
    }
    String::from_utf8(bytes).map_err(|_| invalid("Package URL component is not UTF-8"))
}

/// Decodes an uppercase hexadecimal digit.
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decodes namespace or subpath segments, rejecting empty and encoded-slash segments.
fn segments(value: &str, subpath: bool) -> Result<String> {
    let mut parts = Vec::new();
    for part in value.split('/') {
        let decoded = component(part)?;
        if decoded.contains('/') || subpath && matches!(decoded.as_str(), "." | "..") {
            return Err(invalid("invalid Package URL path segment"));
        }
        parts.push(decoded);
    }
    Ok(parts.join("/"))
}
