// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Validated format conditions evaluated against caller-supplied context.

use crate::{
    Error, ErrorKind, Result,
    cbor::Value,
    container::integer_keys,
    error::invalid,
    version::{JavaRange, JavaVersion},
};

/// Properties of the runtime being considered by a consumer.
#[derive(Clone, Debug)]
pub struct RuntimeContext {
    /// Runtime schema identifier, normally `janex.java`.
    pub runtime_type: String,
    /// Java version when evaluating a Java runtime.
    pub java_version: Option<JavaVersion>,
    /// Exact vendor string reported by the runtime.
    pub vendor: String,
}

/// The environment supplied by the caller; constructing it does not discover or select a runtime.
#[derive(Clone, Debug)]
pub struct Context {
    /// OS token such as `windows`, `linux`, or `macos`.
    pub os: String,
    /// CPU architecture token such as `x86-64` or `aarch64`.
    pub arch: String,
    /// Optional `run`, `open`, or `command` invocation channel.
    pub invocation: Option<String>,
    /// The candidate runtime, if any.
    pub runtime: Option<RuntimeContext>,
}

/// A condition retaining its complete CBOR map, including unknown fields.
#[derive(Clone, Debug)]
pub struct Condition {
    /// The original map.
    value: Value,
    /// Validated Java range, when specified for a Java runtime condition.
    java_range: Option<JavaRange>,
}

impl Condition {
    /// Creates an unconditional empty-map condition.
    pub fn unconditional() -> Self {
        Self {
            value: Value::empty_map(),
            java_range: None,
        }
    }

    /// Validates a condition, including Java requirements even if the condition will not match.
    pub fn from_value(value: Value) -> Result<Self> {
        integer_keys(&value)?;
        for reserved in [0, 3] {
            if value.get(reserved)?.is_some() {
                return Err(invalid("reserved condition key"));
            }
        }
        for key in [1, 2, 4] {
            if let Some(selector) = value.get(key)? {
                names(&selector)?;
            }
        }
        let mut java_range = None;
        if let Some(runtime) = value.get(5)? {
            integer_keys(&runtime)?;
            let runtime_type = runtime.required(0)?;
            nonempty(&runtime_type)?;
            let requirements = runtime.required(1)?;
            integer_keys(&requirements)?;
            if runtime_type.as_text()? == "janex.java" {
                if let Some(version) = requirements.get(0)? {
                    java_range = Some(JavaRange::parse(nonempty(&version)?)?);
                }
                if let Some(vendor) = requirements.get(1)? {
                    nonempty(&vendor)?;
                }
            }
        }
        Ok(Self { value, java_range })
    }

    /// Returns the validated map, preserving unknown fields.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Evaluates known constraints without changing the context or selecting a runtime.
    ///
    /// An unknown runtime type is a mismatch when types differ; matching an unknown
    /// runtime schema reports an unsupported-condition error.
    pub fn matches(&self, context: &Context) -> Result<bool> {
        for (key, actual) in [
            (1, Some(context.os.as_str())),
            (2, Some(context.arch.as_str())),
            (4, context.invocation.as_deref()),
        ] {
            if let Some(selector) = self.value.get(key)? {
                let Some(actual) = actual else {
                    return Ok(false);
                };
                if key == 4 && !matches!(actual, "run" | "open" | "command") {
                    return Ok(false);
                }
                if !names(&selector)?.iter().any(|name| name == actual) {
                    return Ok(false);
                }
            }
        }
        if let Some(runtime) = self.value.get(5)? {
            let Some(actual) = &context.runtime else {
                return Ok(false);
            };
            if runtime.required(0)?.as_text()? != actual.runtime_type {
                return Ok(false);
            }
            if actual.runtime_type != "janex.java" {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "unknown runtime condition schema",
                ));
            }
            if let Some(range) = &self.java_range {
                let version = actual
                    .java_version
                    .as_ref()
                    .ok_or_else(|| invalid("Java runtime context has no version"))?;
                if !range.contains(version) {
                    return Ok(false);
                }
            }
            if let Some(vendor) = runtime.required(1)?.get(1)?
                && vendor.as_text()? != actual.vendor
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Returns the strings in a validated nonempty selector.
fn names(value: &Value) -> Result<Vec<String>> {
    if let Ok(text) = value.as_text() {
        nonempty(value)?;
        return Ok(vec![text.into()]);
    }
    let values = value.as_array()?;
    if values.is_empty() {
        return Err(invalid("empty condition selector"));
    }
    values
        .iter()
        .map(|value| nonempty(value).map(str::to_owned))
        .collect()
}

/// Borrows a schema-required nonempty text value.
pub(crate) fn nonempty(value: &Value) -> Result<&str> {
    let text = value.as_text()?;
    if text.is_empty() {
        return Err(invalid("empty text value"));
    }
    Ok(text)
}
