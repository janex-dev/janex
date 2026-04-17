// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::string_pool::StringPool;

/// A class file in the JVM class file format.
///
/// See [JVM Spec §4.1](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.1).
pub struct ClassFile {
    /// The minor version number stored in the class file header.
    pub minor_version: u16,
    /// The major version number stored in the class file header.
    pub major_version: u16,
    /// The declared number of entries in the constant pool, including the unused slot `0`.
    pub constant_pool_count: u16,
    /// The parsed constant-pool entries in file order.
    pub constant_pool: Box<[ConstantPoolInfo]>,
    /// The class or interface access flags from the class file header.
    pub access_flags: u16,
    /// The constant-pool index of the current class.
    pub this_class: u16,
    /// The constant-pool index of the direct superclass, or `0` for `java/lang/Object`.
    pub super_class: u16,
    /// The constant-pool indices of all directly implemented interfaces.
    pub interfaces: Box<[u16]>,
    /// The declared field members of the class.
    pub fields: Box<[MemberInfo]>,
    /// The declared method members of the class.
    pub methods: Box<[MemberInfo]>,
    /// The declared number of class-level attributes.
    pub attributes_count: u16,
    /// The parsed class-level attributes.
    pub attributes: Box<[AttributeInfo]>,
}

impl ClassFile {
    /// The standard JVM class-file magic number.
    pub const MAGIC_NUMBER: u32 = 0xCAFEBABE;
    /// The magic number used by Janex's transformed class-file payload.
    pub const TRANSFORMED_MAGIC_NUMBER: u32 = 0x70CAFECA;
    /// The synthetic constant-pool tag used by Janex for a shared UTF-8 string.
    pub const TAG_EXTERNAL_UTF8: u8 = 0xFF;
    /// The synthetic constant-pool tag used by Janex for a shared class name split into package and simple name.
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

    /// Reads a field or method table from the class file.
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

    /// Reads an attribute table from the class file.
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
    /// Synthetic padding occupying the slot after a `Long` or `Double`.
    Padding,
    /// The CONSTANT_Utf8_info structure is used to represent constant string values.
    ///
    /// See [JVM Spec §4.4.7](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.7)
    Utf8 {
        /// The byte length of the Modified UTF-8 payload.
        length: u16,
        /// The raw Modified UTF-8 bytes stored by the class file.
        bytes: Box<[u8]>,
    },

    /// The CONSTANT_Integer_info structure represents 4-byte integer constants.
    ///
    /// See [JVM Spec §4.4.4](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.4)
    Integer {
        /// The four raw bytes of the integer constant.
        bytes: u32,
    },

    /// The CONSTANT_Float_info structure represents 4-byte floating-point constants.
    ///
    /// See [JVM Spec §4.4.4](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.4)
    Float {
        /// The four raw bytes of the floating-point constant.
        bytes: u32,
    },

    /// The CONSTANT_Long_info structure represents 8-byte integer constants.
    ///
    /// See [JVM Spec §4.4.5](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.5)
    Long {
        /// The high 32 bits of the 64-bit integer constant.
        high_bytes: u32,
        /// The low 32 bits of the 64-bit integer constant.
        low_bytes: u32,
    },

    /// The CONSTANT_Double_info structure represents 8-byte floating-point constants.
    ///
    /// See [JVM Spec §4.4.5](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.5)
    Double {
        /// The high 32 bits of the 64-bit floating-point constant.
        high_bytes: u32,
        /// The low 32 bits of the 64-bit floating-point constant.
        low_bytes: u32,
    },

    /// The CONSTANT_Class_info structure is used to represent a class or an interface.
    ///
    /// See [JVM Spec §4.4.1](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.1)
    Class {
        /// The constant-pool index of the internal class name stored as a UTF-8 constant.
        name_index: u16,
    },

    /// The CONSTANT_String_info structure is used to represent string constants.
    ///
    /// See [JVM Spec §4.4.3](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.3)
    String {
        /// The constant-pool index of the backing UTF-8 string.
        string_index: u16,
    },

    /// The CONSTANT_Fieldref_info structure is used to represent a field reference.
    ///
    /// See [JVM Spec §4.4.2](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.2)
    Fieldref {
        /// The constant-pool index of the declaring class.
        class_index: u16,
        /// The constant-pool index of the referenced `NameAndType`.
        name_and_type_index: u16,
    },

    /// The CONSTANT_Methodref_info structure is used to represent a method reference.
    ///
    /// See [JVM Spec §4.4.2](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.2)
    Methodref {
        /// The constant-pool index of the declaring class.
        class_index: u16,
        /// The constant-pool index of the referenced `NameAndType`.
        name_and_type_index: u16,
    },

    /// The CONSTANT_InterfaceMethodref_info structure is used to represent an interface method reference.
    ///
    /// See [JVM Spec §4.4.2](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.2)
    InterfaceMethodref {
        /// The constant-pool index of the declaring interface.
        class_index: u16,
        /// The constant-pool index of the referenced `NameAndType`.
        name_and_type_index: u16,
    },

    /// The CONSTANT_NameAndType_info structure is used to represent a field or method, without indicating which class or interface type it belongs to.
    ///
    /// See [JVM Spec §4.4.6](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.6)
    NameAndType {
        /// The constant-pool index of the referenced member name.
        name_index: u16,
        /// The constant-pool index of the referenced descriptor string.
        descriptor_index: u16,
    },

    /// The CONSTANT_MethodHandle_info structure is used to represent a method handle.
    ///
    /// See [JVM Spec §4.4.8](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.8)
    MethodHandle {
        /// The method-handle kind discriminator from the class file.
        reference_kind: u8,
        /// The constant-pool index of the referenced field or method.
        reference_index: u16,
    },

    /// The CONSTANT_MethodType_info structure is used to represent a method descriptor.
    ///
    /// See [JVM Spec §4.4.9](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.9)
    MethodType {
        /// The constant-pool index of the descriptor string.
        descriptor_index: u16,
    },

    /// The CONSTANT_Dynamic_info structure is used to represent a dynamically-computed constant.
    ///
    /// See [JVM Spec §4.4.10](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.10)
    Dynamic {
        /// The index into the BootstrapMethods attribute table.
        bootstrap_method_attr_index: u16,
        /// The constant-pool index of the referenced `NameAndType`.
        name_and_type_index: u16,
    },

    /// The CONSTANT_InvokeDynamic_info structure is used to represent an invokedynamic instruction.
    ///
    /// See [JVM Spec §4.4.10](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.10)
    InvokeDynamic {
        /// The index into the BootstrapMethods attribute table.
        bootstrap_method_attr_index: u16,
        /// The constant-pool index of the referenced `NameAndType`.
        name_and_type_index: u16,
    },

    /// The CONSTANT_Module_info structure is used to represent a module.
    ///
    /// See [JVM Spec §4.4.11](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.11)
    Module {
        /// The constant-pool index of the module name.
        name_index: u16,
    },

    /// The CONSTANT_Package_info structure is used to represent a package.
    ///
    /// See [JVM Spec §4.4.12](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.12)
    Package {
        /// The constant-pool index of the package name.
        name_index: u16,
    },
}

#[allow(non_upper_case_globals)]
impl ConstantPoolInfo {
    /// Raw tag value for `CONSTANT_Utf8_info`.
    pub const TAG_Utf8: u8 = 1;
    /// Raw tag value for `CONSTANT_Integer_info`.
    pub const TAG_Integer: u8 = 3;
    /// Raw tag value for `CONSTANT_Float_info`.
    pub const TAG_Float: u8 = 4;
    /// Raw tag value for `CONSTANT_Long_info`.
    pub const TAG_Long: u8 = 5;
    /// Raw tag value for `CONSTANT_Double_info`.
    pub const TAG_Double: u8 = 6;
    /// Raw tag value for `CONSTANT_Class_info`.
    pub const TAG_Class: u8 = 7;
    /// Raw tag value for `CONSTANT_String_info`.
    pub const TAG_String: u8 = 8;
    /// Raw tag value for `CONSTANT_Fieldref_info`.
    pub const TAG_Fieldref: u8 = 9;
    /// Raw tag value for `CONSTANT_Methodref_info`.
    pub const TAG_Methodref: u8 = 10;
    /// Raw tag value for `CONSTANT_InterfaceMethodref_info`.
    pub const TAG_InterfaceMethodref: u8 = 11;
    /// Raw tag value for `CONSTANT_NameAndType_info`.
    pub const TAG_NameAndType: u8 = 12;
    /// Raw tag value for `CONSTANT_MethodHandle_info`.
    pub const TAG_MethodHandle: u8 = 15;
    /// Raw tag value for `CONSTANT_MethodType_info`.
    pub const TAG_MethodType: u8 = 16;
    /// Raw tag value for `CONSTANT_Dynamic_info`.
    pub const TAG_Dynamic: u8 = 17;
    /// Raw tag value for `CONSTANT_InvokeDynamic_info`.
    pub const TAG_InvokeDynamic: u8 = 18;
    /// Raw tag value for `CONSTANT_Module_info`.
    pub const TAG_Module: u8 = 19;
    /// Raw tag value for `CONSTANT_Package_info`.
    pub const TAG_Package: u8 = 20;

    /// Returns the raw constant-pool tag for this parsed entry.
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

    /// Reads one constant-pool entry from the class-file stream.
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
    /// The JVM access flags of the field or method.
    pub access_flags: u16,
    /// The constant-pool index of the member name.
    pub name_index: u16,
    /// The constant-pool index of the member descriptor.
    pub descriptor_index: u16,
    /// The declared number of attributes attached to this member.
    pub attributes_count: u16,
    /// The parsed member attributes.
    pub attributes: Box<[AttributeInfo]>,
}

/// An attribute in the class file format.
///
/// See [JVM Spec §4.7](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.7).
pub struct AttributeInfo {
    /// The constant-pool index of the attribute name.
    pub attribute_name_index: u16,
    /// The byte length of the attribute payload.
    pub attribute_length: u32,
    /// The raw attribute bytes.
    pub info: Box<[u8]>,
}

/// Rewrites a standard class file into Janex's transformed class-file form and interns eligible strings into the shared `StringPool`.
pub(crate) fn compress_with_string_pool(
    bytes: &[u8],
    string_pool: &mut StringPool,
) -> Result<Vec<u8>, Error> {
    // The sharing decision is made from a parsed view first so references can be classified by semantic role.
    let shared_utf8_kinds = collect_shared_utf8_kinds(&ClassFile::parse_from_bytes(bytes)?)?;
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
    for constant_pool_index in 1..constant_pool_count {
        if skip_slot {
            // `Long` and `Double` consume two constant-pool indices in the JVM format.
            skip_slot = false;
            continue;
        }

        let tag = reader.read_u8()?;
        match tag {
            ConstantPoolInfo::TAG_Utf8 => {
                let length = reader.read_u16_be()? as usize;
                let bytes = reader.read_u8_array(length)?;
                match shared_utf8_kinds[constant_pool_index as usize] {
                    SharedUtf8Kind::NotShared => {
                        // Strings outside Janex's sharing policy stay in standard `CONSTANT_Utf8_info` form.
                        writer.write_u8(ConstantPoolInfo::TAG_Utf8);
                        writer.write_u16_be(length as u16);
                        writer.write_all(bytes.as_ref());
                    }
                    SharedUtf8Kind::Utf8 => {
                        let value = decode_modified_utf8(bytes.as_ref())?;
                        let string_pool_index = string_pool.push(value);
                        writer.write_u8(ClassFile::TAG_EXTERNAL_UTF8);
                        writer.write_vuint(string_pool_index);
                    }
                    SharedUtf8Kind::ClassName => {
                        let value = decode_modified_utf8(bytes.as_ref())?;
                        // Class names share package and simple-name components independently.
                        let (package_name, class_name) = split_class_name(&value);
                        let package_name_index = string_pool.push(package_name);
                        let class_name_index = string_pool.push(class_name);
                        writer.write_u8(ClassFile::TAG_EXTERNAL_UTF8_CLASS);
                        writer.write_vuint(package_name_index);
                        writer.write_vuint(class_name_index);
                    }
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

/// Restores a Janex-transformed class file back to the standard JVM class-file form using the shared `StringPool`.
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
            // The physical bytes were already copied; this iteration is only the logical padding slot.
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
                // Janex external strings are materialized back into standard Modified UTF-8 constants.
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
                // Split package/class entries are rejoined into the JVM internal class-name form.
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

/// Copies a non-shared constant-pool entry verbatim from one class-file stream to another.
fn copy_constant(
    reader: &mut impl DataReader,
    writer: &mut VecDataWriter,
    tag: u8,
) -> Result<(), Error> {
    writer.write_u8(tag);
    match tag {
        ConstantPoolInfo::TAG_Utf8 => {
            let length = reader.read_u16_be()?;
            writer.write_u16_be(length);
            writer.write_all(reader.read_u8_array(length as usize)?.as_ref());
        }
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

/// Decodes a Modified UTF-8 byte sequence from a class file into a Rust `String`.
fn decode_modified_utf8(bytes: &[u8]) -> Result<String, Error> {
    let mut utf16 = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let first = bytes[index];
        if first & 0x80 == 0 {
            // Plain ASCII bytes are encoded directly, except that Modified UTF-8 reserves `0x00`.
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
            // Two-byte sequences cover U+0000 and the rest of the 11-bit range.
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
            // Three-byte sequences encode one UTF-16 code unit; surrogate pairing is handled by `String::from_utf16`.
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

/// Encodes a Rust `str` into the Modified UTF-8 form used by JVM class files.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharedUtf8Kind {
    /// Keep the `Utf8` constant inline in the transformed class file.
    NotShared,
    /// Replace the `Utf8` constant with `TAG_EXTERNAL_UTF8`.
    Utf8,
    /// Replace the `Utf8` constant with `TAG_EXTERNAL_UTF8_CLASS`.
    ClassName,
}

/// Classifies each constant-pool UTF-8 entry according to Janex's sharing policy.
fn collect_shared_utf8_kinds(class_file: &ClassFile) -> Result<Vec<SharedUtf8Kind>, Error> {
    let mut kinds = vec![SharedUtf8Kind::NotShared; class_file.constant_pool_count as usize];
    for constant in class_file.constant_pool.iter() {
        match constant {
            ConstantPoolInfo::Class { name_index } => {
                // Class names always take precedence because they can use the split package/simple-name encoding.
                *kinds.get_mut(*name_index as usize).ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "class name index {} points outside the constant pool",
                        name_index
                    ))
                })? = SharedUtf8Kind::ClassName;
            }
            ConstantPoolInfo::Package { name_index } => {
                let kind = kinds.get_mut(*name_index as usize).ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "package name index {} points outside the constant pool",
                        name_index
                    ))
                })?;
                if *kind != SharedUtf8Kind::ClassName {
                    *kind = SharedUtf8Kind::Utf8;
                }
            }
            ConstantPoolInfo::NameAndType {
                name_index,
                descriptor_index,
            } => {
                // Name-and-type strings share as plain external UTF-8 unless a `Class` entry already claimed them.
                let name_kind = kinds.get_mut(*name_index as usize).ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "name-and-type name index {} points outside the constant pool",
                        name_index
                    ))
                })?;
                if *name_kind != SharedUtf8Kind::ClassName {
                    *name_kind = SharedUtf8Kind::Utf8;
                }
                let descriptor_kind =
                    kinds.get_mut(*descriptor_index as usize).ok_or_else(|| {
                        Error::InvalidReference(format!(
                            "name-and-type descriptor index {} points outside the constant pool",
                            descriptor_index
                        ))
                    })?;
                if *descriptor_kind != SharedUtf8Kind::ClassName {
                    *descriptor_kind = SharedUtf8Kind::Utf8;
                }
            }
            _ => {}
        }
    }
    Ok(kinds)
}

/// Splits an internal JVM class name into package name and simple class name for `CONSTANT_External_String_Class`.
fn split_class_name(value: &str) -> (&str, &str) {
    if let Some((package_name, class_name)) = value.rsplit_once('/')
        && !class_name.is_empty()
    {
        return (package_name, class_name);
    }
    ("", value)
}
