// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::string_pool::StringPool;

/// A class file in the JVM class file format.
///
/// See [JVM Spec §4.1](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.1).
pub struct ClassFile {
    pub minor_version: u16,
    pub major_version: u16,
    pub constant_pool_count: u16,
    pub constant_pool: Box<[ConstantPoolInfo]>,
    pub access_flags: u16,
    pub this_class: u16,
    pub super_class: u16,
    pub interfaces: Box<[u16]>,
    pub fields: Box<[MemberInfo]>,
    pub methods: Box<[MemberInfo]>,
    pub attributes_count: u16,
    pub attributes: Box<[AttributeInfo]>,
}

impl ClassFile {
    /// The magic number for a class file.
    pub const MAGIC_NUMBER: u32 = 0xCAFEBABE;
    /// The magic number for a Janex-transformed class file.
    pub const TRANSFORMED_MAGIC_NUMBER: u32 = 0x70CAFECA;
    pub const TAG_EXTERNAL_UTF8: u8 = 0xFF;
    pub const TAG_EXTERNAL_UTF8_CLASS: u8 = 0xFE;

    /// Parses a class file from the given bytes.
    pub fn parse(reader: &mut impl DataReader) -> Result<ClassFile, Error> {
        let magic = reader.read_u32_be()?;
        if magic != Self::MAGIC_NUMBER {
            return Err(Error::InvalidMagicNumber {
                expected: Self::MAGIC_NUMBER as u64,
                actual: magic as u64,
            });
        }

        let minor_version = reader.read_u16_be()?;
        let major_version = reader.read_u16_be()?;

        let constant_pool_count = reader.read_u16_be()?;
        let constant_pool = Self::read_constant_pool(reader, constant_pool_count)?;

        let access_flags = reader.read_u16_be()?;
        let this_class = reader.read_u16_be()?;
        let super_class = reader.read_u16_be()?;

        let interfaces_count = reader.read_u16_be()?;
        let interfaces = reader.read_u16_array_be(interfaces_count as usize)?;

        let fields_count = reader.read_u16_be()?;
        let fields = Self::read_members(reader, fields_count)?;

        let methods_count = reader.read_u16_be()?;
        let methods = Self::read_members(reader, methods_count)?;

        let attributes_count = reader.read_u16_be()?;
        let attributes = Self::read_attributes(reader, attributes_count)?;

        Ok(ClassFile {
            minor_version,
            major_version,
            constant_pool_count,
            constant_pool,
            access_flags,
            this_class,
            super_class,
            interfaces,
            fields,
            methods,
            attributes_count,
            attributes,
        })
    }

    /// Parses a class file from the given bytes.
    pub fn parse_from_bytes(bytes: &[u8]) -> Result<ClassFile, Error> {
        Self::parse(&mut ArrayDataReader::new(bytes))
    }

    /// Reads the constant pool from the class file.
    fn read_constant_pool(
        reader: &mut impl DataReader,
        constant_pool_count: u16,
    ) -> Result<Box<[ConstantPoolInfo]>, Error> {
        let mut constant_pool = Vec::with_capacity(constant_pool_count as usize);

        for idx in 0..constant_pool_count {
            if idx == 0 {
                constant_pool.push(ConstantPoolInfo::Padding);
            } else {
                match constant_pool[(idx - 1) as usize] {
                    ConstantPoolInfo::Long { .. } | ConstantPoolInfo::Double { .. } => {
                        constant_pool.push(ConstantPoolInfo::Padding)
                    }
                    _ => constant_pool.push(ConstantPoolInfo::read_constant(reader)?),
                }
            }
        }

        Ok(constant_pool.into())
    }

    fn read_members(
        reader: &mut impl DataReader,
        members_count: u16,
    ) -> Result<Box<[MemberInfo]>, Error> {
        let mut fields = Vec::with_capacity(members_count as usize);
        for _ in 0..members_count {
            let access_flags = reader.read_u16_be()?;
            let name_index = reader.read_u16_be()?;
            let descriptor_index = reader.read_u16_be()?;
            let attributes_count = reader.read_u16_be()?;
            let attributes = Self::read_attributes(reader, attributes_count)?;
            fields.push(MemberInfo {
                access_flags,
                name_index,
                descriptor_index,
                attributes_count,
                attributes,
            });
        }
        Ok(fields.into())
    }

    fn read_attributes(
        reader: &mut impl DataReader,
        attributes_count: u16,
    ) -> Result<Box<[AttributeInfo]>, Error> {
        let mut attributes = Vec::with_capacity(attributes_count as usize);
        for _ in 0..attributes_count {
            let attribute_name_index = reader.read_u16_be()?;
            let attribute_length = reader.read_u32_be()?;
            let info = reader.read_u8_array(attribute_length as usize)?;
            attributes.push(AttributeInfo {
                attribute_name_index,
                attribute_length,
                info,
            });
        }
        Ok(attributes.into())
    }
}

/// A constant pool entry in the class file format.
///
/// See [JVM Spec §4.4](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4).
pub enum ConstantPoolInfo {
    Padding,
    /// The CONSTANT_Utf8_info structure is used to represent constant string values.
    ///
    /// See [JVM Spec §4.4.7](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.7)
    Utf8 {
        length: u16,
        bytes: Box<[u8]>,
    },

    /// The CONSTANT_Integer_info structure represents 4-byte integer constants.
    ///
    /// See [JVM Spec §4.4.4](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.4)
    Integer {
        bytes: u32,
    },

    /// The CONSTANT_Float_info structure represents 4-byte floating-point constants.
    ///
    /// See [JVM Spec §4.4.4](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.4)
    Float {
        bytes: u32,
    },

    /// The CONSTANT_Long_info structure represents 8-byte integer constants.
    ///
    /// See [JVM Spec §4.4.5](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.5)
    Long {
        high_bytes: u32,
        low_bytes: u32,
    },

    /// The CONSTANT_Double_info structure represents 8-byte floating-point constants.
    ///
    /// See [JVM Spec §4.4.5](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.5)
    Double {
        high_bytes: u32,
        low_bytes: u32,
    },

    /// The CONSTANT_Class_info structure is used to represent a class or an interface.
    ///
    /// See [JVM Spec §4.4.1](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.1)
    Class {
        name_index: u16,
    },

    /// The CONSTANT_String_info structure is used to represent string constants.
    ///
    /// See [JVM Spec §4.4.3](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.3)
    String {
        string_index: u16,
    },

    /// The CONSTANT_Fieldref_info structure is used to represent a field reference.
    ///
    /// See [JVM Spec §4.4.2](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.2)
    Fieldref {
        class_index: u16,
        name_and_type_index: u16,
    },

    /// The CONSTANT_Methodref_info structure is used to represent a method reference.
    ///
    /// See [JVM Spec §4.4.2](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.2)
    Methodref {
        class_index: u16,
        name_and_type_index: u16,
    },

    /// The CONSTANT_InterfaceMethodref_info structure is used to represent an interface method reference.
    ///
    /// See [JVM Spec §4.4.2](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.2)
    InterfaceMethodref {
        class_index: u16,
        name_and_type_index: u16,
    },

    /// The CONSTANT_NameAndType_info structure is used to represent a field or method, without indicating which class or interface type it belongs to.
    ///
    /// See [JVM Spec §4.4.6](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.6)
    NameAndType {
        name_index: u16,
        descriptor_index: u16,
    },

    /// The CONSTANT_MethodHandle_info structure is used to represent a method handle.
    ///
    /// See [JVM Spec §4.4.8](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.8)
    MethodHandle {
        reference_kind: u8,
        reference_index: u16,
    },

    /// The CONSTANT_MethodType_info structure is used to represent a method descriptor.
    ///
    /// See [JVM Spec §4.4.9](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.9)
    MethodType {
        descriptor_index: u16,
    },

    /// The CONSTANT_Dynamic_info structure is used to represent a dynamically-computed constant.
    ///
    /// See [JVM Spec §4.4.10](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.10)
    Dynamic {
        bootstrap_method_attr_index: u16,
        name_and_type_index: u16,
    },

    /// The CONSTANT_InvokeDynamic_info structure is used to represent an invokedynamic instruction.
    ///
    /// See [JVM Spec §4.4.10](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.10)
    InvokeDynamic {
        bootstrap_method_attr_index: u16,
        name_and_type_index: u16,
    },

    /// The CONSTANT_Module_info structure is used to represent a module.
    ///
    /// See [JVM Spec §4.4.11](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.11)
    Module {
        name_index: u16,
    },

    /// The CONSTANT_Package_info structure is used to represent a package.
    ///
    /// See [JVM Spec §4.4.12](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.12)
    Package {
        name_index: u16,
    },
}

#[allow(non_upper_case_globals)]
impl ConstantPoolInfo {
    pub const TAG_Utf8: u8 = 1;
    pub const TAG_Integer: u8 = 3;
    pub const TAG_Float: u8 = 4;
    pub const TAG_Long: u8 = 5;
    pub const TAG_Double: u8 = 6;
    pub const TAG_Class: u8 = 7;
    pub const TAG_String: u8 = 8;
    pub const TAG_Fieldref: u8 = 9;
    pub const TAG_Methodref: u8 = 10;
    pub const TAG_InterfaceMethodref: u8 = 11;
    pub const TAG_NameAndType: u8 = 12;
    pub const TAG_MethodHandle: u8 = 15;
    pub const TAG_MethodType: u8 = 16;
    pub const TAG_Dynamic: u8 = 17;
    pub const TAG_InvokeDynamic: u8 = 18;
    pub const TAG_Module: u8 = 19;
    pub const TAG_Package: u8 = 20;

    pub const fn tag(&self) -> u8 {
        match self {
            ConstantPoolInfo::Padding => panic!("Padding constant pool entry has no tag"),
            ConstantPoolInfo::Utf8 { .. } => Self::TAG_Utf8,
            ConstantPoolInfo::Integer { .. } => Self::TAG_Integer,
            ConstantPoolInfo::Float { .. } => Self::TAG_Float,
            ConstantPoolInfo::Long { .. } => Self::TAG_Long,
            ConstantPoolInfo::Double { .. } => Self::TAG_Double,
            ConstantPoolInfo::Class { .. } => Self::TAG_Class,
            ConstantPoolInfo::String { .. } => Self::TAG_String,
            ConstantPoolInfo::Fieldref { .. } => Self::TAG_Fieldref,
            ConstantPoolInfo::Methodref { .. } => Self::TAG_Methodref,
            ConstantPoolInfo::InterfaceMethodref { .. } => Self::TAG_InterfaceMethodref,
            ConstantPoolInfo::NameAndType { .. } => Self::TAG_NameAndType,
            ConstantPoolInfo::MethodHandle { .. } => Self::TAG_MethodHandle,
            ConstantPoolInfo::MethodType { .. } => Self::TAG_MethodType,
            ConstantPoolInfo::Dynamic { .. } => Self::TAG_Dynamic,
            ConstantPoolInfo::InvokeDynamic { .. } => Self::TAG_InvokeDynamic,
            ConstantPoolInfo::Module { .. } => Self::TAG_Module,
            ConstantPoolInfo::Package { .. } => Self::TAG_Package,
        }
    }

    pub fn read_constant(reader: &mut impl DataReader) -> Result<ConstantPoolInfo, Error> {
        let tag = reader.read_u8()?;
        Ok(match tag {
            Self::TAG_Utf8 => {
                let length = reader.read_u16_be()?;
                let bytes = reader.read_u8_array(length as usize)?;
                ConstantPoolInfo::Utf8 { length, bytes }
            }
            Self::TAG_Integer => {
                let bytes = reader.read_u32_be()?;
                ConstantPoolInfo::Integer { bytes }
            }
            Self::TAG_Float => {
                let bytes = reader.read_u32_be()?;
                ConstantPoolInfo::Float { bytes }
            }
            Self::TAG_Long => {
                let high_bytes = reader.read_u32_be()?;
                let low_bytes = reader.read_u32_be()?;
                ConstantPoolInfo::Long {
                    high_bytes,
                    low_bytes,
                }
            }
            Self::TAG_Double => {
                let high_bytes = reader.read_u32_be()?;
                let low_bytes = reader.read_u32_be()?;
                ConstantPoolInfo::Double {
                    high_bytes,
                    low_bytes,
                }
            }
            Self::TAG_Class => {
                let name_index = reader.read_u16_be()?;
                ConstantPoolInfo::Class { name_index }
            }
            Self::TAG_String => {
                let string_index = reader.read_u16_be()?;
                ConstantPoolInfo::String { string_index }
            }
            Self::TAG_Fieldref => {
                let class_index = reader.read_u16_be()?;
                let name_and_type_index = reader.read_u16_be()?;
                ConstantPoolInfo::Fieldref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_Methodref => {
                let class_index = reader.read_u16_be()?;
                let name_and_type_index = reader.read_u16_be()?;
                ConstantPoolInfo::Methodref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_InterfaceMethodref => {
                let class_index = reader.read_u16_be()?;
                let name_and_type_index = reader.read_u16_be()?;
                ConstantPoolInfo::InterfaceMethodref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_NameAndType => {
                let name_index = reader.read_u16_be()?;
                let descriptor_index = reader.read_u16_be()?;
                ConstantPoolInfo::NameAndType {
                    name_index,
                    descriptor_index,
                }
            }
            Self::TAG_MethodHandle => {
                let reference_kind = reader.read_u8()?;
                let reference_index = reader.read_u16_be()?;
                ConstantPoolInfo::MethodHandle {
                    reference_kind,
                    reference_index,
                }
            }
            Self::TAG_MethodType => {
                let descriptor_index = reader.read_u16_be()?;
                ConstantPoolInfo::MethodType { descriptor_index }
            }
            Self::TAG_Dynamic => {
                let bootstrap_method_attr_index = reader.read_u16_be()?;
                let name_and_type_index = reader.read_u16_be()?;
                ConstantPoolInfo::Dynamic {
                    bootstrap_method_attr_index,
                    name_and_type_index,
                }
            }
            Self::TAG_InvokeDynamic => {
                let bootstrap_method_attr_index = reader.read_u16_be()?;
                let name_and_type_index = reader.read_u16_be()?;
                ConstantPoolInfo::InvokeDynamic {
                    bootstrap_method_attr_index,
                    name_and_type_index,
                }
            }
            Self::TAG_Module => {
                let name_index = reader.read_u16_be()?;
                ConstantPoolInfo::Module { name_index }
            }
            Self::TAG_Package => {
                let name_index = reader.read_u16_be()?;
                ConstantPoolInfo::Package { name_index }
            }
            _ => {
                return Err(Error::UnknownConstantPoolInfo { tag });
            }
        })
    }
}

/// A field or method in the class file format.
///
/// See [JVM Spec §4.5](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.5)
/// and [JVM Spec §4.6](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.6).
pub struct MemberInfo {
    pub access_flags: u16,
    pub name_index: u16,
    pub descriptor_index: u16,
    pub attributes_count: u16,
    pub attributes: Box<[AttributeInfo]>,
}

/// An attribute in the class file format.
///
/// See [JVM Spec §4.7](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.7).
pub struct AttributeInfo {
    pub attribute_name_index: u16,
    pub attribute_length: u32,
    pub info: Box<[u8]>,
}

pub(crate) fn compress_with_string_pool(
    bytes: &[u8],
    string_pool: &mut StringPool,
) -> Result<Vec<u8>, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = reader.read_u32_be()?;
    if magic != ClassFile::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: ClassFile::MAGIC_NUMBER as u64,
            actual: magic as u64,
        });
    }

    let mut writer = VecDataWriter::new();
    writer.write_u32_be(ClassFile::TRANSFORMED_MAGIC_NUMBER);
    let minor_version = reader.read_u16_be()?;
    let major_version = reader.read_u16_be()?;
    writer.write_u16_be(minor_version);
    writer.write_u16_be(major_version);

    let constant_pool_count = reader.read_u16_be()?;
    writer.write_u16_be(constant_pool_count);

    let mut skip_slot = false;
    for _ in 1..constant_pool_count {
        if skip_slot {
            skip_slot = false;
            continue;
        }

        let tag = reader.read_u8()?;
        match tag {
            ConstantPoolInfo::TAG_Utf8 => {
                let length = reader.read_u16_be()? as usize;
                let bytes = reader.read_u8_array(length)?;
                let value = decode_modified_utf8(bytes.as_ref())?;
                if let Some((package_name, class_name)) = split_package_name(&value) {
                    let package_name_index = string_pool.push(package_name);
                    let class_name_index = string_pool.push(class_name);
                    writer.write_u8(ClassFile::TAG_EXTERNAL_UTF8_CLASS);
                    writer.write_vuint(package_name_index);
                    writer.write_vuint(class_name_index);
                } else {
                    let string_pool_index = string_pool.push(value);
                    writer.write_u8(ClassFile::TAG_EXTERNAL_UTF8);
                    writer.write_vuint(string_pool_index);
                }
            }
            ConstantPoolInfo::TAG_Long | ConstantPoolInfo::TAG_Double => {
                skip_slot = true;
                copy_constant(&mut reader, &mut writer, tag)?;
            }
            _ => copy_constant(&mut reader, &mut writer, tag)?,
        }
    }

    writer.write_all(reader.read_u8_array(reader.remaining())?.as_ref());
    Ok(writer.into_inner())
}

pub(crate) fn decompress_with_string_pool(
    bytes: &[u8],
    string_pool: &StringPool,
) -> Result<Vec<u8>, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = reader.read_u32_be()?;
    if magic != ClassFile::TRANSFORMED_MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: ClassFile::TRANSFORMED_MAGIC_NUMBER as u64,
            actual: magic as u64,
        });
    }

    let mut writer = VecDataWriter::new();
    writer.write_u32_be(ClassFile::MAGIC_NUMBER);
    let minor_version = reader.read_u16_be()?;
    let major_version = reader.read_u16_be()?;
    writer.write_u16_be(minor_version);
    writer.write_u16_be(major_version);

    let constant_pool_count = reader.read_u16_be()?;
    writer.write_u16_be(constant_pool_count);

    let mut skip_slot = false;
    for _ in 1..constant_pool_count {
        if skip_slot {
            skip_slot = false;
            continue;
        }

        let tag = reader.read_u8()?;
        match tag {
            ClassFile::TAG_EXTERNAL_UTF8 => {
                let string_pool_index = reader.read_vuint()?;
                let value = string_pool.get(string_pool_index).ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "invalid string pool index {} in classfile data",
                        string_pool_index
                    ))
                })?;
                let bytes = encode_modified_utf8(value);
                let length = u16::try_from(bytes.len()).map_err(|_| {
                    Error::InvalidValue("modified UTF-8 string is too large for a class file")
                })?;
                writer.write_u8(ConstantPoolInfo::TAG_Utf8);
                writer.write_u16_be(length);
                writer.write_all(&bytes);
            }
            ClassFile::TAG_EXTERNAL_UTF8_CLASS => {
                let package_name_index = reader.read_vuint()?;
                let class_name_index = reader.read_vuint()?;
                let package_name = string_pool.get(package_name_index).ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "invalid string pool index {} in classfile data",
                        package_name_index
                    ))
                })?;
                let class_name = string_pool.get(class_name_index).ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "invalid string pool index {} in classfile data",
                        class_name_index
                    ))
                })?;
                let value = if package_name.is_empty() {
                    class_name.to_string()
                } else {
                    format!("{package_name}/{class_name}")
                };
                let bytes = encode_modified_utf8(&value);
                let length = u16::try_from(bytes.len()).map_err(|_| {
                    Error::InvalidValue("modified UTF-8 string is too large for a class file")
                })?;
                writer.write_u8(ConstantPoolInfo::TAG_Utf8);
                writer.write_u16_be(length);
                writer.write_all(&bytes);
            }
            ConstantPoolInfo::TAG_Long | ConstantPoolInfo::TAG_Double => {
                skip_slot = true;
                copy_constant(&mut reader, &mut writer, tag)?;
            }
            _ => copy_constant(&mut reader, &mut writer, tag)?,
        }
    }

    writer.write_all(reader.read_u8_array(reader.remaining())?.as_ref());
    Ok(writer.into_inner())
}

fn copy_constant(
    reader: &mut impl DataReader,
    writer: &mut VecDataWriter,
    tag: u8,
) -> Result<(), Error> {
    writer.write_u8(tag);
    match tag {
        ConstantPoolInfo::TAG_Integer
        | ConstantPoolInfo::TAG_Float
        | ConstantPoolInfo::TAG_Fieldref
        | ConstantPoolInfo::TAG_Methodref
        | ConstantPoolInfo::TAG_InterfaceMethodref
        | ConstantPoolInfo::TAG_NameAndType
        | ConstantPoolInfo::TAG_Dynamic
        | ConstantPoolInfo::TAG_InvokeDynamic => {
            writer.write_all(reader.read_u8_array(4)?.as_ref());
        }
        ConstantPoolInfo::TAG_Long | ConstantPoolInfo::TAG_Double => {
            writer.write_all(reader.read_u8_array(8)?.as_ref());
        }
        ConstantPoolInfo::TAG_Class
        | ConstantPoolInfo::TAG_String
        | ConstantPoolInfo::TAG_MethodType
        | ConstantPoolInfo::TAG_Module
        | ConstantPoolInfo::TAG_Package => {
            writer.write_all(reader.read_u8_array(2)?.as_ref());
        }
        ConstantPoolInfo::TAG_MethodHandle => {
            writer.write_all(reader.read_u8_array(3)?.as_ref());
        }
        _ => return Err(Error::UnknownConstantPoolInfo { tag }),
    }
    Ok(())
}

fn decode_modified_utf8(bytes: &[u8]) -> Result<String, Error> {
    let mut utf16 = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let first = bytes[index];
        if first & 0x80 == 0 {
            if first == 0 {
                return Err(Error::InvalidValue(
                    "modified UTF-8 must not contain embedded zero bytes",
                ));
            }
            utf16.push(first as u16);
            index += 1;
            continue;
        }

        if first & 0xE0 == 0xC0 {
            if index + 1 >= bytes.len() {
                return Err(Error::InvalidValue("invalid modified UTF-8 string"));
            }
            let second = bytes[index + 1];
            if second & 0xC0 != 0x80 {
                return Err(Error::InvalidValue("invalid modified UTF-8 string"));
            }
            let value = (((first & 0x1F) as u16) << 6) | ((second & 0x3F) as u16);
            if value < 0x80 && !(first == 0xC0 && second == 0x80) {
                return Err(Error::InvalidValue("invalid modified UTF-8 string"));
            }
            utf16.push(value);
            index += 2;
            continue;
        }

        if first & 0xF0 == 0xE0 {
            if index + 2 >= bytes.len() {
                return Err(Error::InvalidValue("invalid modified UTF-8 string"));
            }
            let second = bytes[index + 1];
            let third = bytes[index + 2];
            if second & 0xC0 != 0x80 || third & 0xC0 != 0x80 {
                return Err(Error::InvalidValue("invalid modified UTF-8 string"));
            }
            let value = (((first & 0x0F) as u16) << 12)
                | (((second & 0x3F) as u16) << 6)
                | ((third & 0x3F) as u16);
            if value < 0x0800 {
                return Err(Error::InvalidValue("invalid modified UTF-8 string"));
            }
            utf16.push(value);
            index += 3;
            continue;
        }

        return Err(Error::InvalidValue("invalid modified UTF-8 string"));
    }

    String::from_utf16(&utf16).map_err(|_| Error::InvalidValue("invalid modified UTF-8 string"))
}

fn encode_modified_utf8(value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for code_unit in value.encode_utf16() {
        match code_unit {
            0x0001..=0x007F => bytes.push(code_unit as u8),
            0x0000 | 0x0080..=0x07FF => {
                bytes.push(0xC0 | (((code_unit >> 6) & 0x1F) as u8));
                bytes.push(0x80 | ((code_unit & 0x3F) as u8));
            }
            _ => {
                bytes.push(0xE0 | ((code_unit >> 12) as u8));
                bytes.push(0x80 | (((code_unit >> 6) & 0x3F) as u8));
                bytes.push(0x80 | ((code_unit & 0x3F) as u8));
            }
        }
    }
    bytes
}

fn split_package_name(value: &str) -> Option<(&str, &str)> {
    let (package_name, class_name) = value.rsplit_once('/')?;
    if package_name.is_empty() || class_name.is_empty() {
        return None;
    }
    Some((package_name, class_name))
}
