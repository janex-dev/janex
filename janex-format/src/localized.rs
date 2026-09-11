// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Localized display text with BCP 47 language lookup.

use crate::{Result, cbor::Value, error::invalid};
use std::collections::BTreeSet;

/// A bare string or translations retaining their original deterministic CBOR representation.
#[derive(Clone, Debug)]
pub struct LocalizedText {
    /// Original value, including malformed language keys that lookup ignores.
    value: Value,
    /// Usable translations in deterministic encoded-key order, with lowercase tags.
    translations: Vec<(String, String)>,
}

impl LocalizedText {
    /// Validates display text, optionally requiring every translation to be nonempty.
    ///
    /// Map keys and values must be strings. Malformed language tags are ignored during
    /// lookup, but at least one well-formed tag is required. Well-formed tags differing
    /// only in ASCII case are duplicates. Registry membership is not required.
    pub fn from_value(value: Value, require_nonempty: bool) -> Result<Self> {
        let mut translations = Vec::new();
        if let Ok(text) = value.as_text() {
            if require_nonempty && text.is_empty() {
                return Err(invalid("empty localized display name"));
            }
        } else {
            let mut seen = BTreeSet::new();
            for (tag, text) in value.as_map()? {
                let tag = tag.as_text()?;
                let text = text.as_text()?;
                if require_nonempty && text.is_empty() {
                    return Err(invalid("empty localized display name"));
                }
                if well_formed(tag) {
                    let tag = tag.to_ascii_lowercase();
                    if !seen.insert(tag.clone()) {
                        return Err(invalid("duplicate language tag"));
                    }
                    translations.push((tag, text.into()));
                }
            }
            if translations.is_empty() {
                return Err(invalid("localized text has no well-formed language tag"));
            }
        }
        Ok(Self {
            value,
            translations,
        })
    }

    /// Returns the complete original value, including ignored language keys.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Selects text using RFC 4647 lookup, then `und`, then deterministic map order.
    ///
    /// A malformed locale skips lookup and uses the fallback. When several tags match
    /// a lookup range, their original deterministic map order breaks the tie.
    pub fn select(&self, locale: &str) -> &str {
        if let Ok(text) = self.value.as_text() {
            return text;
        }
        if well_formed(locale) {
            let mut range = locale.to_ascii_lowercase();
            while !range.is_empty() {
                if let Some((_, text)) = self.translations.iter().find(|(tag, _)| {
                    tag == &range
                        || tag
                            .strip_prefix(&range)
                            .is_some_and(|rest| rest.starts_with('-'))
                }) {
                    return text;
                }
                let Some((parent, _)) = range.rsplit_once('-') else {
                    break;
                };
                let mut end = parent.len();
                if parent
                    .rsplit('-')
                    .next()
                    .is_some_and(|part| part.len() == 1)
                {
                    end = parent.rfind('-').unwrap_or(0);
                }
                range.truncate(end);
            }
        }
        self.translations
            .iter()
            .find(|(tag, _)| tag == "und")
            .unwrap_or(&self.translations[0])
            .1
            .as_str()
    }
}

/// Checks BCP 47 syntax, including private-use subtag lengths.
fn well_formed(tag: &str) -> bool {
    tag.split('-').all(|part| (1..=8).contains(&part.len()))
        && language_tags::LanguageTag::parse(tag).is_ok()
}
