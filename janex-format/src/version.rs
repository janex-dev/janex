// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Java version comparison and canonical `vers:jep322` constraint timelines.

use crate::{Result, error::invalid};
use std::{cmp::Ordering, fmt};

/// A Java 8-or-later version, compared without build or optional information.
///
/// Numeric elements are compared with missing trailing elements treated as zero.
/// Prereleases sort before the corresponding release. Java 8 aliases are expanded
/// before comparison; build numbers do not distinguish versions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JavaVersion {
    /// Numeric version elements with trailing zeroes removed.
    numbers: Vec<u32>,
    /// Prerelease identifier, with numeric leading zeroes removed.
    pre: Option<String>,
}

impl JavaVersion {
    /// Parses a runtime version or one of the Java 8 aliases in the format specification.
    ///
    /// Equivalent trailing zero elements are accepted here; range parsing separately
    /// requires canonical written version numbers. Invalid suffixes and Java 7 or older
    /// versions are rejected.
    pub fn parse(text: &str) -> Result<Self> {
        parse(text).map(|(version, _)| version)
    }

    /// Returns the Java feature version, such as 8, 17, or 25.
    pub fn feature(&self) -> u32 {
        self.numbers[0]
    }
}

impl Ord for JavaVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        for index in 0..self.numbers.len().max(other.numbers.len()) {
            let order = self
                .numbers
                .get(index)
                .unwrap_or(&0)
                .cmp(other.numbers.get(index).unwrap_or(&0));
            if order != Ordering::Equal {
                return order;
            }
        }
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => match (numeric(a), numeric(b)) {
                (true, true) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => a.cmp(b),
            },
        }
    }
}

impl PartialOrd for JavaVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for JavaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, number) in self.numbers.iter().enumerate() {
            if index != 0 {
                f.write_str(".")?;
            }
            write!(f, "{number}")?;
        }
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

/// One comparator on a VERS version timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Comparator {
    /// Includes only this point without changing surrounding intervals.
    Equal,
    /// Excludes this point without changing surrounding intervals.
    NotEqual,
    /// Starts an interval after this point.
    Greater,
    /// Starts an interval including this point.
    GreaterEqual,
    /// Ends an interval before this point.
    Less,
    /// Ends an interval including this point.
    LessEqual,
}

impl Comparator {
    /// Returns whether this comparator starts an interval.
    fn greater(self) -> bool {
        matches!(self, Self::Greater | Self::GreaterEqual)
    }
    /// Returns whether this comparator ends an interval.
    fn less(self) -> bool {
        matches!(self, Self::Less | Self::LessEqual)
    }
}

/// A validated canonical VERS timeline for Java versions.
#[derive(Clone, Debug)]
pub struct JavaRange {
    /// Original spelling, retained for metadata and diagnostics.
    text: String,
    /// Strictly increasing version signposts; empty denotes `*`.
    constraints: Vec<(Comparator, JavaVersion)>,
}

impl JavaRange {
    /// Parses a canonical `vers:jep322/` range, rejecting duplicate versions and invalid timelines.
    pub fn parse(text: &str) -> Result<Self> {
        let body = text
            .strip_prefix("vers:jep322/")
            .ok_or_else(|| invalid("expected a vers:jep322 range"))?;
        if !text.bytes().all(|byte| (33..=126).contains(&byte)) {
            return Err(invalid("VERS must use printable ASCII without whitespace"));
        }
        if body == "*" {
            return Ok(Self {
                text: text.into(),
                constraints: Vec::new(),
            });
        }
        let mut constraints = Vec::new();
        let mut previous_non_exclusion: Option<Comparator> = None;
        let mut previous_boundary: Option<Comparator> = None;
        for item in body.split('|') {
            let (comparator, version) = [
                (">=", Comparator::GreaterEqual),
                ("<=", Comparator::LessEqual),
                ("!=", Comparator::NotEqual),
                (">", Comparator::Greater),
                ("<", Comparator::Less),
                ("=", Comparator::Equal),
            ]
            .into_iter()
            .find_map(|(prefix, comparator)| {
                item.strip_prefix(prefix)
                    .map(|version| (comparator, version))
            })
            .unwrap_or((Comparator::Equal, item));
            let (version, canonical) = parse(version)?;
            if !canonical {
                return Err(invalid("noncanonical Java version in VERS"));
            }
            if constraints
                .last()
                .is_some_and(|(_, previous)| previous >= &version)
            {
                return Err(invalid("VERS versions must be unique and increasing"));
            }
            if comparator != Comparator::NotEqual {
                if previous_non_exclusion == Some(Comparator::Equal) && comparator.less() {
                    return Err(invalid(
                        "VERS equality cannot be followed by an upper bound",
                    ));
                }
                previous_non_exclusion = Some(comparator);
            }
            if comparator.greater() || comparator.less() {
                if previous_boundary
                    .is_some_and(|previous| previous.greater() == comparator.greater())
                {
                    return Err(invalid("VERS lower and upper bounds must alternate"));
                }
                previous_boundary = Some(comparator);
            }
            constraints.push((comparator, version));
        }
        Ok(Self {
            text: text.into(),
            constraints,
        })
    }

    /// Returns the accepted VERS spelling without rewriting aliases or optional version information.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Tests a version against the intervals and explicitly included or excluded points.
    pub fn contains(&self, candidate: &JavaVersion) -> bool {
        if self.constraints.is_empty() {
            return true;
        }
        let meaningful = self
            .constraints
            .iter()
            .find(|(comparator, _)| *comparator != Comparator::NotEqual);
        let mut included = meaningful.is_none_or(|(comparator, _)| comparator.less());
        for (comparator, version) in &self.constraints {
            match candidate.cmp(version) {
                Ordering::Less => break,
                Ordering::Equal => {
                    return matches!(
                        comparator,
                        Comparator::Equal | Comparator::GreaterEqual | Comparator::LessEqual
                    );
                }
                Ordering::Greater => {
                    if comparator.greater() {
                        included = true;
                    }
                    if comparator.less() {
                        included = false;
                    }
                }
            }
        }
        included
    }
}

/// Parses a version and reports whether its written numeric sequence is canonical for a range.
fn parse(text: &str) -> Result<(JavaVersion, bool)> {
    if text.is_empty() || !text.is_ascii() {
        return Err(invalid("invalid Java version"));
    }
    if let Some(alias) = text.strip_prefix("8u") {
        decimal(alias)?;
        let (version, _) = parse(&format!("8.0.{alias}"))?;
        return Ok((version, true));
    }
    if let Some(alias) = text.strip_prefix("1.8.0") {
        let canonical = if let Some(update) = alias.strip_prefix('_') {
            let (number, suffix) = update
                .split_once('-')
                .map_or((update, None), |(number, suffix)| (number, Some(suffix)));
            decimal(number)?;
            if let Some(build) = suffix
                .and_then(|suffix| suffix.strip_prefix('b'))
                .filter(|build| numeric(build))
            {
                decimal(build)?;
                format!("8.0.{number}")
            } else if let Some(pre) = suffix {
                validate_pre(pre)?;
                format!("8.0.{number}-{pre}")
            } else {
                format!("8.0.{number}")
            }
        } else if let Some(pre) = alias.strip_prefix('-') {
            validate_pre(pre)?;
            format!("8-{pre}")
        } else if alias.is_empty() {
            "8".into()
        } else {
            return Err(invalid("invalid Java 8 alias"));
        };
        let (version, _) = parse(&canonical)?;
        return Ok((version, true));
    }
    let (before_build, build) = text
        .split_once('+')
        .map_or((text, None), |(a, b)| (a, Some(b)));
    let mut prefix = before_build.splitn(3, '-');
    let number = prefix.next().expect("split has a first element");
    let pre = prefix.next();
    let optional = prefix.next();
    let mut numbers = number.split('.').map(decimal).collect::<Result<Vec<_>>>()?;
    if numbers[0] < 8 {
        return Err(invalid("Java feature version must be at least 8"));
    }
    let canonical = numbers.len() == 1 || numbers.last() != Some(&0);
    while numbers.len() > 1 && numbers.last() == Some(&0) {
        numbers.pop();
    }
    if let Some(pre) = pre {
        validate_pre(pre)?;
    }
    if let Some(optional) = optional {
        validate_optional(optional)?;
    }
    if let Some(build) = build {
        if optional.is_some() {
            return Err(invalid(
                "optional version information must follow the build",
            ));
        }
        let (build, optional) = build
            .split_once('-')
            .map_or((build, None), |(a, b)| (a, Some(b)));
        if build.is_empty() {
            if pre.is_some() || optional.is_none() {
                return Err(invalid("empty Java build number"));
            }
        } else {
            decimal(build)?;
        }
        if let Some(optional) = optional {
            validate_optional(optional)?;
        }
    }
    let pre = pre.map(|pre| {
        if numeric(pre) {
            let value = pre.trim_start_matches('0');
            if value.is_empty() {
                "0".into()
            } else {
                value.into()
            }
        } else {
            pre.into()
        }
    });
    Ok((JavaVersion { numbers, pre }, canonical))
}

/// Parses a bounded decimal integer without unnecessary leading zeroes.
fn decimal(text: &str) -> Result<u32> {
    if !numeric(text) || (text.len() > 1 && text.starts_with('0')) {
        return Err(invalid("invalid Java version number"));
    }
    let value = text
        .parse::<u32>()
        .map_err(|_| invalid("Java version number overflow"))?;
    if value > i32::MAX as u32 {
        return Err(invalid("Java version number exceeds Runtime.Version range"));
    }
    Ok(value)
}

/// Returns whether a nonempty string consists entirely of ASCII digits.
fn numeric(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Checks a nonempty alphanumeric prerelease identifier.
fn validate_pre(text: &str) -> Result<()> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(invalid("invalid Java prerelease identifier"));
    }
    Ok(())
}

/// Checks additional build information according to the Java version grammar.
fn validate_optional(text: &str) -> Result<()> {
    if text.is_empty()
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
    {
        return Err(invalid("invalid Java optional version information"));
    }
    Ok(())
}
