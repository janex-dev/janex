// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::io::DataReader;

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

    /// Parses a class file from the given bytes.
    pub fn parse(bytes: &[u8]) -> Result<ClassFile, Error> {
        let mut reader = DataReader::new(bytes);

        let magic = reader.read_u32()?;
        if magic != Self::MAGIC_NUMBER {
            return Err(Error::InvalidMagicNumber(magic));
        }

        let minor_version = reader.read_u16()?;
        let major_version = reader.read_u16()?;

        let constant_pool_count = reader.read_u16()?;
        let constant_pool = Self::read_constant_pool(&mut reader, constant_pool_count)?;

        let access_flags = reader.read_u16()?;
        let this_class = reader.read_u16()?;
        let super_class = reader.read_u16()?;

        let interfaces_count = reader.read_u16()?;
        let interfaces = reader.read_u16_array(interfaces_count as usize)?;

        let fields_count = reader.read_u16()?;
        let fields = Self::read_members(&mut reader, fields_count)?;

        let methods_count = reader.read_u16()?;
        let methods = Self::read_members(&mut reader, methods_count)?;

        let attributes_count = reader.read_u16()?;
        let attributes = Self::read_attributes(&mut reader, attributes_count)?;

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

    /// Reads the constant pool from the class file.
    fn read_constant_pool(
        reader: &mut DataReader,
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
        reader: &mut DataReader,
        members_count: u16,
    ) -> Result<Box<[MemberInfo]>, Error> {
        let mut fields = Vec::with_capacity(members_count as usize);
        for _ in 0..members_count {
            let access_flags = reader.read_u16()?;
            let name_index = reader.read_u16()?;
            let descriptor_index = reader.read_u16()?;
            let attributes_count = reader.read_u16()?;
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
        reader: &mut DataReader,
        attributes_count: u16,
    ) -> Result<Box<[AttributeInfo]>, Error> {
        let mut attributes = Vec::with_capacity(attributes_count as usize);
        for _ in 0..attributes_count {
            let attribute_name_index = reader.read_u16()?;
            let attribute_length = reader.read_u32()?;
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

    pub fn read_constant(reader: &mut DataReader) -> Result<ConstantPoolInfo, Error> {
        let tag = reader.read_u8()?;
        Ok(match tag {
            Self::TAG_Utf8 => {
                let length = reader.read_u16()?;
                let bytes = reader.read_u8_array(length as usize)?;
                ConstantPoolInfo::Utf8 { length, bytes }
            }
            Self::TAG_Integer => {
                let bytes = reader.read_u32()?;
                ConstantPoolInfo::Integer { bytes }
            }
            Self::TAG_Float => {
                let bytes = reader.read_u32()?;
                ConstantPoolInfo::Float { bytes }
            }
            Self::TAG_Long => {
                let high_bytes = reader.read_u32()?;
                let low_bytes = reader.read_u32()?;
                ConstantPoolInfo::Long {
                    high_bytes,
                    low_bytes,
                }
            }
            Self::TAG_Double => {
                let high_bytes = reader.read_u32()?;
                let low_bytes = reader.read_u32()?;
                ConstantPoolInfo::Double {
                    high_bytes,
                    low_bytes,
                }
            }
            Self::TAG_Class => {
                let name_index = reader.read_u16()?;
                ConstantPoolInfo::Class { name_index }
            }
            Self::TAG_String => {
                let string_index = reader.read_u16()?;
                ConstantPoolInfo::String { string_index }
            }
            Self::TAG_Fieldref => {
                let class_index = reader.read_u16()?;
                let name_and_type_index = reader.read_u16()?;
                ConstantPoolInfo::Fieldref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_Methodref => {
                let class_index = reader.read_u16()?;
                let name_and_type_index = reader.read_u16()?;
                ConstantPoolInfo::Methodref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_InterfaceMethodref => {
                let class_index = reader.read_u16()?;
                let name_and_type_index = reader.read_u16()?;
                ConstantPoolInfo::InterfaceMethodref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_NameAndType => {
                let name_index = reader.read_u16()?;
                let descriptor_index = reader.read_u16()?;
                ConstantPoolInfo::NameAndType {
                    name_index,
                    descriptor_index,
                }
            }
            Self::TAG_MethodHandle => {
                let reference_kind = reader.read_u8()?;
                let reference_index = reader.read_u16()?;
                ConstantPoolInfo::MethodHandle {
                    reference_kind,
                    reference_index,
                }
            }
            Self::TAG_MethodType => {
                let descriptor_index = reader.read_u16()?;
                ConstantPoolInfo::MethodType { descriptor_index }
            }
            Self::TAG_Dynamic => {
                let bootstrap_method_attr_index = reader.read_u16()?;
                let name_and_type_index = reader.read_u16()?;
                ConstantPoolInfo::Dynamic {
                    bootstrap_method_attr_index,
                    name_and_type_index,
                }
            }
            Self::TAG_InvokeDynamic => {
                let bootstrap_method_attr_index = reader.read_u16()?;
                let name_and_type_index = reader.read_u16()?;
                ConstantPoolInfo::InvokeDynamic {
                    bootstrap_method_attr_index,
                    name_and_type_index,
                }
            }
            Self::TAG_Module => {
                let name_index = reader.read_u16()?;
                ConstantPoolInfo::Module { name_index }
            }
            Self::TAG_Package => {
                let name_index = reader.read_u16()?;
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

/// Errors that can occur when parsing a class file.
pub enum Error {
    UnexpectedEndOfFile,
    InvalidMagicNumber(u32),
    UnknownConstantPoolInfo { tag: u8 },
}
