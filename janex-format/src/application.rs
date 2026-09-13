// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Application descriptors and Java launch configuration evaluation.

use crate::{
    Error, ErrorKind, Result,
    binary::{Decoder, Limits},
    blob::BlobRef,
    cbor::{self, Value},
    checksum::Checksum,
    condition::{Condition, Context, nonempty},
    container::{APPLICATION, Reader, integer_keys},
    error::invalid,
    localized::LocalizedText,
};
use std::{
    collections::BTreeSet,
    io::{Read, Seek},
};

/// A Java entry point selected from the classpath or one named module.
#[derive(Clone, Debug)]
pub struct EntryPoint {
    /// Binary class name; absent when the named module supplies its main class.
    pub main_class: Option<String>,
    /// Exact module name; absent for classpath launching.
    pub main_module: Option<String>,
}

impl EntryPoint {
    /// Reads an entry point with at least one nonempty class or module name.
    fn from_value(value: &Value) -> Result<Self> {
        integer_keys(value)?;
        let main_class = optional_text(value, 0)?;
        let main_module = optional_text(value, 1)?;
        if main_class.is_none() && main_module.is_none() {
            return Err(invalid("Java entry point has no class or module"));
        }
        Ok(Self {
            main_class,
            main_module,
        })
    }

    /// Encodes an entry point, rejecting absent or empty names.
    pub fn to_value(&self) -> Result<Value> {
        let mut fields = Vec::new();
        if let Some(name) = &self.main_class {
            fields.push((Value::uint(0), Value::text(name)));
        }
        if let Some(name) = &self.main_module {
            fields.push((Value::uint(1), Value::text(name)));
        }
        let value = Value::map(fields)?;
        Self::from_value(&value)?;
        Ok(value)
    }
}

/// One local resource root or external URI in an ordered Java path.
#[derive(Clone, Debug)]
pub enum PathEntry {
    /// A resource-root blob in the same container.
    Local(BlobRef),
    /// An external resource or virtual Java module requirement.
    External {
        /// Exact validated URI; `pkg:` values are canonical Package URLs.
        uri: String,
        /// Optional checksum over content returned by a resolver.
        checksum: Option<Checksum>,
    },
}

impl PathEntry {
    /// Validates a path entry and whether a virtual Janex module is permitted here.
    pub fn from_value(value: &Value, module_path: bool) -> Result<Self> {
        integer_keys(value)?;
        match value.required(0)?.as_u64()? {
            0 => Ok(Self::Local(BlobRef::from_value(&value.required(1)?)?)),
            1 => {
                let uri = value.required(1)?;
                let uri = nonempty(&uri)?;
                validate_uri(uri, module_path)?;
                let checksum = value
                    .get(2)?
                    .map(|checksum| Checksum::decode(checksum.as_byte_string()?))
                    .transpose()?;
                Ok(Self::External {
                    uri: uri.into(),
                    checksum,
                })
            }
            _ => Err(invalid("unknown Java path entry type")),
        }
    }

    /// Encodes a local or external path entry, validating its permitted location.
    pub fn to_value(&self, module_path: bool) -> Result<Value> {
        let value = match self {
            Self::Local(reference) => Value::map([
                (Value::uint(0), Value::uint(0)),
                (Value::uint(1), reference.to_value()),
            ])?,
            Self::External { uri, checksum } => {
                let mut fields = vec![
                    (Value::uint(0), Value::uint(1)),
                    (Value::uint(1), Value::text(uri)),
                ];
                if let Some(checksum) = checksum {
                    fields.push((Value::uint(2), Value::bytes(&checksum.encode())));
                }
                Value::map(fields)?
            }
        };
        Self::from_value(&value, module_path)?;
        Ok(value)
    }

    /// Returns the exact module name and optional version of a virtual Janex requirement.
    pub fn module_requirement(&self) -> Option<(String, Option<String>)> {
        let Self::External { uri, .. } = self else {
            return None;
        };
        let purl = crate::purl::parse(uri).ok()?;
        (purl.ty() == "janex" && purl.namespace() == Some("java-module"))
            .then(|| (purl.name().into(), purl.version().map(str::to_owned)))
    }
}

/// A Java instrumentation agent and its unsplit option string.
#[derive(Clone, Debug)]
pub struct JavaAgent {
    /// Local or externally resolved agent JAR; virtual module requirements are invalid.
    pub reference: PathEntry,
    /// Empty means no option; otherwise passed as one complete agent option.
    pub option: String,
}

impl JavaAgent {
    /// Reads the required reference and option without treating an empty option as absent.
    fn from_value(value: &Value) -> Result<Self> {
        integer_keys(value)?;
        Ok(Self {
            reference: PathEntry::from_value(&value.required(0)?, false)?,
            option: value.required(1)?.as_text()?.into(),
        })
    }
}

/// An evaluated launch configuration for one candidate runtime and host context.
#[derive(Clone, Debug)]
pub struct JavaLaunch {
    /// Final nonempty entry point after applying all matching overlays.
    pub entry_point: EntryPoint,
    /// Ordered module-path entries, including virtual requirements.
    pub module_path: Vec<PathEntry>,
    /// Ordered classpath entries.
    pub class_path: Vec<PathEntry>,
    /// Agents in application order.
    pub agents: Vec<JavaAgent>,
    /// JVM options, each preserving its complete argument boundary.
    pub jvm_options: Vec<String>,
    /// Preset program arguments, to precede user-supplied arguments.
    pub arguments: Vec<String>,
}

/// One optional list contribution from a configuration object.
#[derive(Clone, Debug)]
enum ListChange<T> {
    /// Missing field: retain the current list.
    Unchanged,
    /// Explicit null: remove every current item.
    Clear,
    /// Append the given items, including an empty no-op contribution.
    Append(Vec<T>),
}

impl<T: Clone> ListChange<T> {
    /// Applies one list contribution within the evaluated collection limit.
    fn apply(&self, target: &mut Vec<T>, limits: Limits) -> Result<()> {
        match self {
            Self::Unchanged => {}
            Self::Clear => target.clear(),
            Self::Append(items) => {
                limits.elements(target.len() as u64 + items.len() as u64)?;
                target.extend_from_slice(items);
            }
        }
        Ok(())
    }
}

/// A validated Java configuration tree, independent of runtime discovery.
#[derive(Clone, Debug)]
struct LaunchConfig {
    /// Condition gating this object and its entire overlay subtree.
    condition: Condition,
    /// Missing outer option retains the entry; present inner None clears it.
    entry_point: Option<Option<EntryPoint>>,
    /// Module-path list contribution.
    module_path: ListChange<PathEntry>,
    /// Classpath list contribution.
    class_path: ListChange<PathEntry>,
    /// Agent list contribution.
    agents: ListChange<JavaAgent>,
    /// JVM argument contribution.
    jvm_options: ListChange<String>,
    /// Program argument contribution.
    arguments: ListChange<String>,
    /// Nested configurations in visitation order.
    overlays: Vec<Self>,
}

impl LaunchConfig {
    /// Parses and validates every subtree before evaluating any conditions.
    fn parse(value: &Value, limits: Limits, depth: usize) -> Result<Self> {
        if depth > limits.max_depth {
            return Err(Error::new(
                ErrorKind::Limit,
                "overlay nesting limit exceeded",
            ));
        }
        integer_keys(value)?;
        let condition = value
            .get(0)?
            .map(Condition::from_value)
            .transpose()?
            .unwrap_or_else(Condition::unconditional);
        let entry_point = value
            .get(1)?
            .map(|entry| {
                if entry.is_null() {
                    Ok(None)
                } else {
                    EntryPoint::from_value(&entry).map(Some)
                }
            })
            .transpose()?;
        let overlays = match value.get(6)? {
            None => Vec::new(),
            Some(overlays) => {
                let overlays = overlays.as_array()?;
                limits.elements(overlays.len() as u64)?;
                overlays
                    .iter()
                    .map(|value| Self::parse(value, limits, depth + 1))
                    .collect::<Result<_>>()?
            }
        };
        Ok(Self {
            condition,
            entry_point,
            module_path: list(value, 2, limits, |value| PathEntry::from_value(value, true))?,
            class_path: list(value, 3, limits, |value| {
                PathEntry::from_value(value, false)
            })?,
            agents: list(value, 4, limits, JavaAgent::from_value)?,
            jvm_options: list(value, 5, limits, |value| Ok(value.as_text()?.into()))?,
            arguments: list(value, 7, limits, |value| Ok(value.as_text()?.into()))?,
            overlays,
        })
    }

    /// Evaluates a matching root in depth-first pre-order, pruning unmatched subtrees.
    fn evaluate(&self, context: &Context, limits: Limits) -> Result<Option<JavaLaunch>> {
        if !self.condition.matches(context)? {
            return Ok(None);
        }
        let mut entry_point = None;
        let mut result = JavaLaunch {
            entry_point: EntryPoint {
                main_class: None,
                main_module: None,
            },
            module_path: Vec::new(),
            class_path: Vec::new(),
            agents: Vec::new(),
            jvm_options: Vec::new(),
            arguments: Vec::new(),
        };
        let mut pending = vec![self];
        while let Some(config) = pending.pop() {
            if !config.condition.matches(context)? {
                continue;
            }
            if let Some(entry) = &config.entry_point {
                entry_point = entry.clone();
            }
            config.module_path.apply(&mut result.module_path, limits)?;
            config.class_path.apply(&mut result.class_path, limits)?;
            config.agents.apply(&mut result.agents, limits)?;
            config.jvm_options.apply(&mut result.jvm_options, limits)?;
            config.arguments.apply(&mut result.arguments, limits)?;
            limits.elements(pending.len() as u64 + config.overlays.len() as u64)?;
            pending.extend(config.overlays.iter().rev());
        }
        result.entry_point =
            entry_point.ok_or_else(|| invalid("evaluated Java launch has no entry point"))?;
        Ok(Some(result))
    }
}

/// A validated application section retaining all original CBOR fields.
#[derive(Clone, Debug)]
pub struct Application {
    /// Original type-specific section metadata.
    type_info: Value,
    /// Original application body, retaining all nested extensions.
    value: Value,
    /// File-local application identity.
    id: String,
    /// Descriptor schema identifier.
    application_type: String,
    /// Optional localized display name.
    name: Option<LocalizedText>,
    /// Optional localized comment.
    comment: Option<LocalizedText>,
    /// Optional command name used as a display-title fallback.
    command: Option<String>,
    /// Whether the application requests a launch without a console window.
    windowed: bool,
    /// Validated Java configuration, absent for unsupported application types.
    launch: Option<LaunchConfig>,
    /// Limits used for parsing and evaluated lists.
    limits: Limits,
}

impl Application {
    /// Validates type metadata, presentation fields, and the complete supported descriptor.
    ///
    /// Unknown application types retain an opaque integer-keyed descriptor. Installation
    /// requests are checked structurally; platform filename policy belongs to the Host.
    pub fn from_values(type_info: Value, value: Value, limits: Limits) -> Result<Self> {
        // Constructors can assemble values without reader limits; enforce them at this boundary.
        Value::from_bytes(type_info.as_bytes(), limits)?;
        Value::from_bytes(value.as_bytes(), limits)?;
        integer_keys(&type_info)?;
        integer_keys(&value)?;
        let id = nonempty(&type_info.required(0)?)?.to_owned();
        let application_type = nonempty(&type_info.required(1)?)?.to_owned();
        let descriptor = value.required(0)?;
        integer_keys(&descriptor)?;
        let name = value
            .get(1)?
            .map(|value| LocalizedText::from_value(value, true))
            .transpose()?;
        let comment = value
            .get(3)?
            .map(|value| LocalizedText::from_value(value, false))
            .transpose()?;
        optional_text(&value, 2)?;
        let command = value
            .get(4)?
            .map(|integration| {
                validate_integration(&integration)?;
                optional_text(&integration, 0)
            })
            .transpose()?
            .flatten();
        let mode = value
            .get(5)?
            .map(|mode| mode.as_u64())
            .transpose()?
            .unwrap_or(0);
        if mode > 1 {
            return Err(invalid("invalid application launch mode"));
        }
        let launch = if application_type == "janex.java" {
            Some(LaunchConfig::parse(&descriptor.required(0)?, limits, 0)?)
        } else {
            None
        };
        Ok(Self {
            type_info,
            value,
            id,
            application_type,
            name,
            comment,
            command,
            windowed: mode == 1,
            launch,
            limits,
        })
    }

    /// Reads one complete application section, including its magic and Sized CBOR body.
    pub fn decode(bytes: &[u8], type_info: Value, limits: Limits) -> Result<Self> {
        let mut decoder = Decoder::new(bytes, limits)?;
        if decoder.u64()? != APPLICATION {
            return Err(invalid("incorrect application section magic"));
        }
        let value = cbor::read_sized(&mut decoder)?;
        decoder.finish()?;
        Self::from_values(type_info, value, limits)
    }

    /// Encodes the complete section, preserving its original deterministic CBOR values.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = APPLICATION.to_le_bytes().to_vec();
        cbor::write_sized(&mut bytes, &self.value)?;
        Ok(bytes)
    }

    /// Returns the file-local application ID.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Returns the application descriptor's schema identifier.
    pub fn application_type(&self) -> &str {
        &self.application_type
    }
    /// Returns the original section type metadata, including extensions.
    pub fn type_info(&self) -> &Value {
        &self.type_info
    }
    /// Returns the original application object, including all nested extensions.
    pub fn value(&self) -> &Value {
        &self.value
    }
    /// Returns whether this application requests windowed launching.
    pub fn windowed(&self) -> bool {
        self.windowed
    }
    /// Returns a localized display title, falling back to the command and then application ID.
    pub fn title(&self, locale: &str) -> &str {
        if let Some(name) = &self.name {
            return name.select(locale);
        }
        self.command.as_deref().unwrap_or(&self.id)
    }
    /// Returns the localized comment, or None when omitted; an empty comment remains present.
    pub fn comment(&self, locale: &str) -> Option<&str> {
        self.comment.as_ref().map(|comment| comment.select(locale))
    }
    /// Evaluates Java configuration; None means this candidate fails the root condition.
    ///
    /// Unsupported application types and matching configurations without an entry point fail.
    pub fn evaluate_java(&self, context: &Context) -> Result<Option<JavaLaunch>> {
        self.launch
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Unsupported, "unsupported application type"))?
            .evaluate(context, self.limits)
    }
}

/// Loads all application sections in section order and rejects duplicate application IDs.
///
/// Reading and schema validation do not authenticate the container or resolve resource blobs.
pub fn read_applications<R: Read + Seek>(reader: &mut Reader<R>) -> Result<Vec<Application>> {
    let sections: Vec<_> = reader
        .sections()
        .filter(|section| section.kind() == APPLICATION)
        .cloned()
        .collect();
    let mut applications = Vec::new();
    let mut ids = BTreeSet::new();
    for section in sections {
        let type_info = section
            .type_info()?
            .ok_or_else(|| invalid("application section has no type_info"))?;
        let application = Application::decode(
            &reader.read_section(section.id())?,
            type_info,
            reader.limits(),
        )?;
        if !ids.insert(application.id.clone()) {
            return Err(invalid("duplicate application ID"));
        }
        applications.push(application);
    }
    Ok(applications)
}

/// Selects an explicit ID, or the sole application when no ID was supplied.
pub fn select_application<'a>(
    applications: &'a [Application],
    id: Option<&str>,
) -> Result<&'a Application> {
    if let Some(id) = id {
        return applications
            .iter()
            .find(|app| app.id == id)
            .ok_or_else(|| invalid(format!("application not found: {id}")));
    }
    match applications {
        [application] => Ok(application),
        [] => Err(invalid("container has no application")),
        _ => Err(invalid(
            "multiple applications require an explicit selection",
        )),
    }
}

/// Reads an optional schema-required nonempty string.
fn optional_text(value: &Value, key: u64) -> Result<Option<String>> {
    value
        .get(key)?
        .map(|value| nonempty(&value).map(str::to_owned))
        .transpose()
}

/// Reads a missing, cleared, or appended typed list.
fn list<T>(
    value: &Value,
    key: u64,
    limits: Limits,
    parse: impl Fn(&Value) -> Result<T>,
) -> Result<ListChange<T>> {
    match value.get(key)? {
        None => Ok(ListChange::Unchanged),
        Some(value) if value.is_null() => Ok(ListChange::Clear),
        Some(value) => {
            let items = value.as_array()?;
            limits.elements(items.len() as u64)?;
            Ok(ListChange::Append(
                items.iter().map(parse).collect::<Result<_>>()?,
            ))
        }
    }
}

/// Checks installation-request framing without applying platform policy.
fn validate_integration(value: &Value) -> Result<()> {
    integer_keys(value)?;
    if let Some(command) = optional_text(value, 0)?
        && (command.contains(['/', '\0']) || matches!(command.as_str(), "." | ".."))
    {
        return Err(invalid("application command must be a single filename"));
    }
    if let Some(desktop) = value.get(1)? {
        desktop.as_bool()?;
    }
    if let Some(icons) = value.get(2)? {
        let icons = icons.as_array()?;
        if icons.is_empty() {
            return Err(invalid("empty application icon array"));
        }
        for icon in icons {
            integer_keys(&icon)?;
            nonempty(&icon.required(0)?)?;
            BlobRef::from_value(&icon.required(1)?)?;
        }
    }
    Ok(())
}

/// Checks URI syntax, canonical Package URLs, and Janex module-requirement placement.
fn validate_uri(uri: &str, module_path: bool) -> Result<()> {
    let parsed =
        fluent_uri::Uri::parse(uri).map_err(|_| invalid("invalid external resource URI"))?;
    if parsed.scheme().as_str().eq_ignore_ascii_case("pkg") {
        let purl = crate::purl::parse(uri)?;
        if purl.ty() == "janex"
            && (!module_path
                || purl.namespace() != Some("java-module")
                || !purl.qualifiers().is_empty()
                || purl.subpath().is_some())
        {
            return Err(invalid(
                "Janex Java module requirement is invalid in this path",
            ));
        }
    }
    Ok(())
}
