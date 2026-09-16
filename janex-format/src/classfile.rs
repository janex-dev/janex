// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Lossless constant-pool string sharing and Java class/module descriptor inspection.
//!
//! Transformation checks only constant-pool framing. Explicit inspection also checks
//! references, code boundaries, and module descriptors. The JVM validates loaded classes.

use crate::{
    Result,
    binary::{Decoder, Limits, write_vuint},
    data_pool::DataPool,
    error::invalid,
};
use std::{borrow::Cow, collections::BTreeSet};

/// A module dependency declared in a Module attribute.
#[derive(Clone, Debug)]
pub struct ModuleRequirement {
    /// Required module name.
    pub name: String,
    /// Version recorded when compiling the descriptor, if present.
    pub compiled_version: Option<String>,
    /// JVM requires flags; bit `0x0040` denotes a static-phase requirement.
    pub flags: u16,
}

/// Module metadata needed to construct and check a Java launch.
#[derive(Clone, Debug)]
pub struct ModuleInfo {
    /// Module name.
    pub name: String,
    /// Exact descriptor version, if present.
    pub version: Option<String>,
    /// Binary main-class name supplied by ModuleMainClass, if present.
    pub main_class: Option<String>,
    /// Requirements in descriptor order.
    pub requires: Vec<ModuleRequirement>,
}

/// Structural information extracted from an ordinary Java class file.
#[derive(Clone, Debug)]
pub struct ClassInfo {
    /// Class-file major version.
    pub major: u16,
    /// Class-file minor version, including the preview marker when present.
    pub minor: u16,
    /// Slash-separated internal class name.
    pub name: String,
    /// Module descriptor when this is a module-info class.
    pub module: Option<ModuleInfo>,
}

/// A borrowed constant-pool record, excluding its tag.
#[derive(Clone, Copy)]
struct Constant<'a> {
    /// JVM constant-pool tag.
    tag: u8,
    /// The exact record body, including a UTF-8 length when applicable.
    body: &'a [u8],
}

/// Parses one ordinary class file, rejecting trailing bytes and malformed structural references.
pub fn inspect(bytes: &[u8], limits: Limits) -> Result<ClassInfo> {
    parse(bytes, limits)
}

/// Produces a smaller transformed class file, or returns `None` without modifying the pool.
///
/// Entries are externalized only when their Modified UTF-8 bytes are also valid UTF-8
/// and the final transformation saves the minimum transform-descriptor overhead.
/// Existing constant-pool indices and body bytes are retained.
/// Class-file internals are not validated beyond constant-pool framing.
pub fn transform(bytes: &[u8], strings: &mut DataPool, limits: Limits) -> Result<Option<Vec<u8>>> {
    let (constants, body_start) = scan_pool(bytes, limits)?;
    let checkpoint = strings.len();
    let classes: BTreeSet<_> = constants
        .iter()
        .flatten()
        .filter(|constant| constant.tag == 7)
        .map(|constant| be_u16(constant.body, 0))
        .collect();
    let mut output = bytes[..10].to_vec();
    output[..4].copy_from_slice(&[0xca, 0xfe, 0xca, 0x70]);
    for (index, constant) in constants.iter().enumerate().skip(1) {
        let Some(constant) = constant else {
            continue;
        };
        let mut replacement = Vec::new();
        if constant.tag == 1 {
            let original = &constant.body[2..];
            if let Ok(text) = std::str::from_utf8(original)
                && text.chars().all(|ch| ch != '\0' && ch <= '\u{ffff}')
            {
                let before = strings.len();
                if classes.contains(&(index as u16))
                    && !text.starts_with('[')
                    && !text.starts_with('/')
                {
                    let (package, name) = text.rsplit_once('/').unwrap_or(("", text));
                    if !name.is_empty() {
                        replacement.push(0xfe);
                        write_vuint(&mut replacement, strings.intern(package))?;
                        write_vuint(&mut replacement, strings.intern(name))?;
                    }
                } else {
                    replacement.push(0xff);
                    write_vuint(&mut replacement, strings.intern(text))?;
                }
                if replacement.len() > constant.body.len() {
                    strings.truncate(before);
                    replacement.clear();
                }
            }
        }
        if replacement.is_empty() {
            output.push(constant.tag);
            output.extend_from_slice(constant.body);
        } else {
            output.extend(replacement);
        }
    }
    output.extend_from_slice(&bytes[body_start..]);
    let mut descriptor_size = Vec::new();
    write_vuint(&mut descriptor_size, bytes.len() as u64)?;
    if output.len() + descriptor_size.len() + 2 >= bytes.len() {
        strings.truncate(checkpoint);
        return Ok(None);
    }
    Ok(Some(output))
}

/// Restores an ordinary class file from a Janex CLASSFILE transform using the selected pool.
///
/// Checks constant-pool framing, external indices, and encoded string lengths.
/// The remaining class body and Modified UTF-8 contents are copied without validation.
/// Output allocation is bounded by `limits.max_bytes`; callers check the declared decoded size.
pub fn restore(bytes: &[u8], strings: &DataPool, limits: Limits) -> Result<Vec<u8>> {
    let mut decoder = Decoder::new(bytes, limits)?;
    if decoder.take(4)? != [0xca, 0xfe, 0xca, 0x70] {
        return Err(invalid("incorrect transformed class magic"));
    }
    decoder.take(4)?;
    let count = read_u16(&mut decoder)?;
    if count == 0 {
        return Err(invalid("empty class constant-pool count"));
    }
    limits.elements(u64::from(count))?;
    let mut output = bytes[..10].to_vec();
    output[..4].copy_from_slice(&[0xca, 0xfe, 0xba, 0xbe]);
    let mut index = 1;
    while index < count {
        let tag = decoder.u8()?;
        match tag {
            0xff | 0xfe => {
                let utf = if tag == 0xff {
                    let text = strings.get(decoder.vuint()?)?;
                    check_external_length(text.len())?;
                    Cow::Borrowed(text)
                } else {
                    let package = strings.get(decoder.vuint()?)?;
                    let name = strings.get(decoder.vuint()?)?;
                    if name.is_empty() {
                        return Err(invalid("empty external class name"));
                    }
                    check_external_length(
                        package.len() + name.len() + usize::from(!package.is_empty()),
                    )?;
                    if package.is_empty() {
                        Cow::Borrowed(name)
                    } else {
                        let mut bytes = package.to_vec();
                        bytes.push(b'/');
                        bytes.extend_from_slice(name);
                        Cow::Owned(bytes)
                    }
                };
                let length = u16::try_from(utf.len())
                    .map_err(|_| invalid("external class string exceeds 65535 bytes"))?;
                limits.bytes(output.len() as u64 + 3 + utf.len() as u64)?;
                output.push(1);
                output.extend_from_slice(&length.to_be_bytes());
                output.extend_from_slice(&utf);
            }
            _ => {
                let start = decoder.position();
                read_constant(tag, &mut decoder)?;
                limits.bytes(output.len() as u64 + 1 + (decoder.position() - start) as u64)?;
                output.push(tag);
                output.extend_from_slice(&bytes[start..decoder.position()]);
            }
        }
        if matches!(tag, 5 | 6) {
            index += 1;
            if index >= count {
                return Err(invalid("long or double lacks its reserved slot"));
            }
        }
        index += 1;
    }
    limits.bytes(output.len() as u64 + decoder.remaining() as u64)?;
    output.extend_from_slice(decoder.take(decoder.remaining())?);
    Ok(output)
}

/// Scans only the constant-pool framing required for a lossless transformation.
fn scan_pool(bytes: &[u8], limits: Limits) -> Result<(Vec<Option<Constant<'_>>>, usize)> {
    let mut decoder = Decoder::new(bytes, limits)?;
    if decoder.take(4)? != [0xca, 0xfe, 0xba, 0xbe] {
        return Err(invalid("incorrect class magic"));
    }
    decoder.take(4)?;
    let count = usize::from(read_u16(&mut decoder)?);
    limits.elements(count as u64)?;
    if count == 0 {
        return Err(invalid("constant-pool count must include slot zero"));
    }
    let mut constants = vec![None];
    while constants.len() < count {
        let tag = decoder.u8()?;
        let start = decoder.position();
        read_constant(tag, &mut decoder)?;
        constants.push(Some(Constant {
            tag,
            body: &bytes[start..decoder.position()],
        }));
        if matches!(tag, 5 | 6) {
            if constants.len() == count {
                return Err(invalid("long or double lacks its reserved slot"));
            }
            constants.push(None);
        }
    }
    let body_start = decoder.position();
    Ok((constants, body_start))
}

/// Parses and explicitly validates constant-pool references and the framed class body.
fn parse(bytes: &[u8], limits: Limits) -> Result<ClassInfo> {
    let (constants, body_start) = scan_pool(bytes, limits)?;
    let minor = be_u16(bytes, 4);
    let major = be_u16(bytes, 6);
    if major < 45 || (major >= 56 && minor != 0 && minor != 65535) {
        return Err(invalid("invalid class-file version"));
    }
    check_constants(&constants, major)?;
    let mut decoder = Decoder::new(bytes, limits)?;
    decoder.take(body_start)?;
    let flags = read_u16(&mut decoder)?;
    let name = indirect_text(&constants, read_u16(&mut decoder)?, 7)?;
    let superclass = read_u16(&mut decoder)?;
    if superclass != 0 {
        constant(&constants, superclass, &[7])?;
    } else if name != "java/lang/Object" && flags & 0x8000 == 0 {
        return Err(invalid("missing superclass"));
    }
    let interfaces = read_u16(&mut decoder)?;
    for _ in 0..interfaces {
        constant(&constants, read_u16(&mut decoder)?, &[7])?;
    }
    let fields = members(&mut decoder, &constants, false)?;
    let methods = members(&mut decoder, &constants, true)?;
    let attributes = attributes(&mut decoder, &constants, AttributeContext::Class)?;
    decoder.finish()?;
    let mut module = attributes.module;
    if let Some(main) = attributes.main_class {
        module
            .as_mut()
            .ok_or_else(|| invalid("ModuleMainClass requires a module descriptor"))?
            .main_class = Some(main.replace('/', "."));
    }
    if flags & 0x8000 != 0 {
        if major < 53
            || name != "module-info"
            || superclass != 0
            || interfaces != 0
            || fields != 0
            || methods != 0
            || module.is_none()
        {
            return Err(invalid("invalid module-info class structure"));
        }
    } else if module.is_some() {
        return Err(invalid("Module attribute requires ACC_MODULE"));
    }
    for item in constants
        .iter()
        .flatten()
        .filter(|item| matches!(item.tag, 17 | 18))
    {
        if attributes
            .bootstrap_count
            .is_none_or(|count| be_u16(item.body, 0) >= count)
        {
            return Err(invalid(
                "dynamic constant references a missing bootstrap method",
            ));
        }
    }
    Ok(ClassInfo {
        major,
        minor,
        name,
        module,
    })
}

/// Consumes one ordinary constant-pool body without interpreting its contents.
fn read_constant(tag: u8, decoder: &mut Decoder<'_>) -> Result<()> {
    match tag {
        1 => {
            let length = usize::from(read_u16(decoder)?);
            decoder.take(length)?;
        }
        3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => {
            decoder.take(4)?;
        }
        5 | 6 => {
            decoder.take(8)?;
        }
        7 | 8 | 16 | 19 | 20 => {
            decoder.take(2)?;
        }
        15 => {
            decoder.take(3)?;
        }
        _ => return Err(invalid(format!("unknown class constant tag {tag}"))),
    }
    Ok(())
}

/// Checks the expected type of every constant-pool reference.
fn check_constants(constants: &[Option<Constant<'_>>], major: u16) -> Result<()> {
    for item in constants.iter().flatten() {
        match item.tag {
            1 => {
                decode_modified(&item.body[2..])?;
            }
            7 | 8 | 16 | 19 | 20 => {
                constant(constants, be_u16(item.body, 0), &[1])?;
            }
            9..=11 => {
                constant(constants, be_u16(item.body, 0), &[7])?;
                constant(constants, be_u16(item.body, 2), &[12])?;
            }
            12 => {
                constant(constants, be_u16(item.body, 0), &[1])?;
                constant(constants, be_u16(item.body, 2), &[1])?;
            }
            17 | 18 => {
                constant(constants, be_u16(item.body, 2), &[12])?;
            }
            15 => {
                let allowed: &[u8] = match item.body[0] {
                    1..=4 => &[9],
                    5 | 8 => &[10],
                    6 | 7 if major >= 52 => &[10, 11],
                    6 | 7 => &[10],
                    9 => &[11],
                    _ => return Err(invalid("invalid method-handle kind")),
                };
                constant(constants, be_u16(item.body, 1), allowed)?;
            }
            _ => {}
        }
        if (matches!(item.tag, 15 | 16 | 18) && major < 51)
            || (matches!(item.tag, 19 | 20) && major < 53)
            || (item.tag == 17 && major < 55)
        {
            return Err(invalid(
                "constant tag is unavailable in this class-file version",
            ));
        }
    }
    Ok(())
}

/// Returns a usable constant-pool slot with one of the requested tags.
fn constant<'a>(
    constants: &'a [Option<Constant<'a>>],
    index: u16,
    tags: &[u8],
) -> Result<Constant<'a>> {
    constants
        .get(usize::from(index))
        .copied()
        .flatten()
        .filter(|item| tags.contains(&item.tag))
        .ok_or_else(|| invalid("invalid class constant-pool reference"))
}

/// Decodes a UTF-8 constant into Unicode scalar values when the caller needs text.
fn text(constants: &[Option<Constant<'_>>], index: u16) -> Result<String> {
    let value = constant(constants, index, &[1])?;
    String::from_utf16(&decode_modified(&value.body[2..])?)
        .map_err(|_| invalid("unpaired surrogate in class identifier"))
}

/// Resolves a Class, Module, or Package constant to its text.
fn indirect_text(constants: &[Option<Constant<'_>>], index: u16, tag: u8) -> Result<String> {
    let value = constant(constants, index, &[tag])?;
    text(constants, be_u16(value.body, 0))
}

/// Reads an optional UTF-8 constant index, with zero denoting absence.
fn optional_text(constants: &[Option<Constant<'_>>], index: u16) -> Result<Option<String>> {
    if index == 0 {
        Ok(None)
    } else {
        text(constants, index).map(Some)
    }
}

/// Parses field or method framing and attribute lengths.
fn members(
    decoder: &mut Decoder<'_>,
    constants: &[Option<Constant<'_>>],
    methods: bool,
) -> Result<u16> {
    let count = read_u16(decoder)?;
    for _ in 0..count {
        let flags = read_u16(decoder)?;
        constant(constants, read_u16(decoder)?, &[1])?;
        constant(constants, read_u16(decoder)?, &[1])?;
        let attributes = attributes(
            decoder,
            constants,
            if methods {
                AttributeContext::Method
            } else {
                AttributeContext::Field
            },
        )?;
        if methods && attributes.code != (flags & (0x0100 | 0x0400) == 0) {
            return Err(invalid(
                "method Code attribute disagrees with native or abstract flags",
            ));
        }
    }
    Ok(count)
}

/// The class structure in which attributes occur.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AttributeContext {
    /// Attributes of the class itself.
    Class,
    /// Attributes of a field.
    Field,
    /// Attributes of a method.
    Method,
    /// Attributes nested in method code.
    Code,
}

/// Selected attributes used for framing validation and launch metadata.
#[derive(Default)]
struct Attributes {
    /// A Module attribute, if present.
    module: Option<ModuleInfo>,
    /// A ModuleMainClass attribute, if present.
    main_class: Option<String>,
    /// Number of bootstrap methods, if the attribute is present.
    bootstrap_count: Option<u16>,
    /// Whether a Code attribute was present.
    code: bool,
}

/// Checks attribute boundaries while interpreting only attributes required by this codec.
fn attributes(
    decoder: &mut Decoder<'_>,
    constants: &[Option<Constant<'_>>],
    context: AttributeContext,
) -> Result<Attributes> {
    let mut result = Attributes::default();
    for _ in 0..read_u16(decoder)? {
        let name = text(constants, read_u16(decoder)?)?;
        let length = read_u32(decoder)?;
        let limits = decoder.limits();
        let length = limits.bytes(u64::from(length))?;
        let mut value = Decoder::new(decoder.take(length)?, limits)?;
        match name.as_str() {
            "Module" => {
                if context != AttributeContext::Class || result.module.is_some() {
                    return Err(invalid("misplaced or duplicate Module attribute"));
                }
                result.module = Some(read_module(&mut value, constants)?);
            }
            "ModuleMainClass" => {
                if context != AttributeContext::Class || result.main_class.is_some() {
                    return Err(invalid("misplaced or duplicate ModuleMainClass attribute"));
                }
                result.main_class = Some(indirect_text(constants, read_u16(&mut value)?, 7)?);
            }
            "BootstrapMethods" => {
                if context != AttributeContext::Class || result.bootstrap_count.is_some() {
                    return Err(invalid("misplaced or duplicate BootstrapMethods attribute"));
                }
                let count = read_u16(&mut value)?;
                result.bootstrap_count = Some(count);
                for _ in 0..count {
                    constant(constants, read_u16(&mut value)?, &[15])?;
                    for _ in 0..read_u16(&mut value)? {
                        constant(
                            constants,
                            read_u16(&mut value)?,
                            &[3, 4, 5, 6, 7, 8, 15, 16, 17],
                        )?;
                    }
                }
            }
            "Code" => {
                if context != AttributeContext::Method || result.code {
                    return Err(invalid("misplaced or duplicate Code attribute"));
                }
                result.code = true;
                value.take(4)?;
                let length = read_u32(&mut value)?;
                if !(1..65536).contains(&length) {
                    return Err(invalid("invalid method code length"));
                }
                value.take(length as usize)?;
                for _ in 0..read_u16(&mut value)? {
                    let start = read_u16(&mut value)?;
                    let end = read_u16(&mut value)?;
                    let handler = read_u16(&mut value)?;
                    let catch = read_u16(&mut value)?;
                    if start >= end || u32::from(end) > length || u32::from(handler) >= length {
                        return Err(invalid("invalid exception-handler range"));
                    }
                    if catch != 0 {
                        constant(constants, catch, &[7])?;
                    }
                }
                attributes(&mut value, constants, AttributeContext::Code)?;
            }
            _ => {
                value.take(value.remaining())?;
            }
        }
        value.finish()?;
    }
    Ok(result)
}

/// Reads a Module attribute and checks all of its constant-pool references.
fn read_module(
    decoder: &mut Decoder<'_>,
    constants: &[Option<Constant<'_>>],
) -> Result<ModuleInfo> {
    let name = indirect_text(constants, read_u16(decoder)?, 19)?;
    decoder.take(2)?;
    let version = optional_text(constants, read_u16(decoder)?)?;
    let mut requires = Vec::new();
    for _ in 0..read_u16(decoder)? {
        let name = indirect_text(constants, read_u16(decoder)?, 19)?;
        let flags = read_u16(decoder)?;
        let compiled_version = optional_text(constants, read_u16(decoder)?)?;
        requires.push(ModuleRequirement {
            name,
            compiled_version,
            flags,
        });
    }
    for _ in 0..2 {
        for _ in 0..read_u16(decoder)? {
            constant(constants, read_u16(decoder)?, &[20])?;
            decoder.take(2)?;
            for _ in 0..read_u16(decoder)? {
                constant(constants, read_u16(decoder)?, &[19])?;
            }
        }
    }
    for _ in 0..read_u16(decoder)? {
        constant(constants, read_u16(decoder)?, &[7])?;
    }
    for _ in 0..read_u16(decoder)? {
        constant(constants, read_u16(decoder)?, &[7])?;
        let count = read_u16(decoder)?;
        if count == 0 {
            return Err(invalid("empty module provider list"));
        }
        for _ in 0..count {
            constant(constants, read_u16(decoder)?, &[7])?;
        }
    }
    Ok(ModuleInfo {
        name,
        version,
        main_class: None,
        requires,
    })
}

/// Decodes canonical Modified UTF-8 into UTF-16 units, preserving unpaired surrogates.
fn decode_modified(bytes: &[u8]) -> Result<Vec<u16>> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let (count, mut value) = match first {
            1..=0x7f => (1, u16::from(first)),
            0xc0..=0xdf => (2, u16::from(first & 0x1f)),
            0xe0..=0xef => (3, u16::from(first & 0x0f)),
            _ => return Err(invalid("invalid Modified UTF-8 leading byte")),
        };
        let continuation = bytes
            .get(index + 1..index + count)
            .ok_or_else(|| invalid("truncated Modified UTF-8"))?;
        for byte in continuation {
            if byte & 0xc0 != 0x80 {
                return Err(invalid("invalid Modified UTF-8 continuation"));
            }
            value = (value << 6) | u16::from(byte & 0x3f);
        }
        if (count == 2 && value != 0 && value < 0x80) || (count == 3 && value < 0x800) {
            return Err(invalid("overlong Modified UTF-8"));
        }
        result.push(value);
        index += count;
    }
    Ok(result)
}

/// Rejects restored bytes that exceed the CONSTANT_Utf8 length field.
fn check_external_length(length: usize) -> Result<()> {
    if length > usize::from(u16::MAX) {
        return Err(invalid("external class string exceeds 65535 bytes"));
    }
    Ok(())
}

/// Reads a big-endian class-file integer from a bounded decoder.
fn read_u16(decoder: &mut Decoder<'_>) -> Result<u16> {
    Ok(u16::from_be_bytes(
        decoder.take(2)?.try_into().expect("fixed length"),
    ))
}
/// Reads a big-endian class-file integer from a bounded decoder.
fn read_u32(decoder: &mut Decoder<'_>) -> Result<u32> {
    Ok(u32::from_be_bytes(
        decoder.take(4)?.try_into().expect("fixed length"),
    ))
}
/// Reads an integer within an already bounded constant-pool record.
fn be_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("bounded constant"),
    )
}
