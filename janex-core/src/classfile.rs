// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

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
    pub fields: Box<[u16]>,
    pub methods: Box<[u16]>,
    pub attributes: Box<[AttributeInfo]>,
}

impl ClassFile {
    /// The magic number for a class file.
    pub const MAGIC_NUMBER: u32 = 0xCAFEBABE;

    /// Parses a class file from the given bytes.
    pub fn parse(bytes: &[u8]) -> Result<ClassFile, Error> {
        let mut parser = ClassFileParser { bytes };

        let magic = parser.read_u32()?;
        if magic != Self::MAGIC_NUMBER {
            return Err(Error::InvalidMagicNumber(magic));
        }

        let minor_version = parser.read_u16()?;
        let major_version = parser.read_u16()?;

        let constant_pool_count = parser.read_u16()?;
        let constant_pool = Self::read_constant_pool(&mut parser, constant_pool_count)?;

        let access_flags = parser.read_u16()?;
        let this_class = parser.read_u16()?;
        let super_class = parser.read_u16()?;

        let interfaces_count = parser.read_u16()?;
        let interfaces = parser.read_u16_array(interfaces_count as usize)?;

        let fields_count = parser.read_u16()?;
        let fields = parser.read_u16_array(fields_count as usize)?;

        let methods_count = parser.read_u16()?;
        let methods = parser.read_u16_array(methods_count as usize)?;

        let attributes_count = parser.read_u16()?;
        let attributes = Self::read_attributes(&mut parser, attributes_count)?;

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
            attributes,
        })
    }

    /// Reads the constant pool from the class file.
    fn read_constant_pool(
        parser: &mut ClassFileParser,
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
                    _ => constant_pool.push(ConstantPoolInfo::read_constant(parser)?),
                }
            }
        }

        Ok(constant_pool.into())
    }

    fn read_attributes(
        parser: &mut ClassFileParser,
        attributes_count: u16,
    ) -> Result<Box<[AttributeInfo]>, Error> {
        let mut attributes = Vec::with_capacity(attributes_count as usize);
        for _ in 0..attributes_count {
            let attribute_name_index = parser.read_u16()?;
            let attribute_length = parser.read_u32()?;
            let info = parser.read_u8_array(attribute_length as usize)?;
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
    Utf8 {
        length: u16,
        bytes: Box<[u8]>,
    },
    Integer {
        bytes: u32,
    },
    Float {
        bytes: u32,
    },
    Long {
        high_bytes: u32,
        low_bytes: u32,
    },
    Double {
        high_bytes: u32,
        low_bytes: u32,
    },
    Class {
        name_index: u16,
    },
    String {
        string_index: u16,
    },
    Fieldref {
        class_index: u16,
        name_and_type_index: u16,
    },
    Methodref {
        class_index: u16,
        name_and_type_index: u16,
    },
    InterfaceMethodref {
        class_index: u16,
        name_and_type_index: u16,
    },
    NameAndType {
        name_index: u16,
        descriptor_index: u16,
    },
    MethodHandle {
        reference_kind: u8,
        reference_index: u16,
    },
    MethodType {
        descriptor_index: u16,
    },
    Dynamic {
        bootstrap_method_attr_index: u16,
        name_and_type_index: u16,
    },
    InvokeDynamic {
        bootstrap_method_attr_index: u16,
        name_and_type_index: u16,
    },
    Module {
        name_index: u16,
    },
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

    pub fn read_constant(parser: &mut ClassFileParser) -> Result<ConstantPoolInfo, Error> {
        let tag = parser.read_u8()?;
        Ok(match tag {
            Self::TAG_Utf8 => {
                let length = parser.read_u16()?;
                let bytes = parser.read_u8_array(length as usize)?;
                ConstantPoolInfo::Utf8 { length, bytes }
            }
            Self::TAG_Integer => {
                let bytes = parser.read_u32()?;
                ConstantPoolInfo::Integer { bytes }
            }
            Self::TAG_Float => {
                let bytes = parser.read_u32()?;
                ConstantPoolInfo::Float { bytes }
            }
            Self::TAG_Long => {
                let high_bytes = parser.read_u32()?;
                let low_bytes = parser.read_u32()?;
                ConstantPoolInfo::Long {
                    high_bytes,
                    low_bytes,
                }
            }
            Self::TAG_Double => {
                let high_bytes = parser.read_u32()?;
                let low_bytes = parser.read_u32()?;
                ConstantPoolInfo::Double {
                    high_bytes,
                    low_bytes,
                }
            }
            Self::TAG_Class => {
                let name_index = parser.read_u16()?;
                ConstantPoolInfo::Class { name_index }
            }
            Self::TAG_String => {
                let string_index = parser.read_u16()?;
                ConstantPoolInfo::String { string_index }
            }
            Self::TAG_Fieldref => {
                let class_index = parser.read_u16()?;
                let name_and_type_index = parser.read_u16()?;
                ConstantPoolInfo::Fieldref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_Methodref => {
                let class_index = parser.read_u16()?;
                let name_and_type_index = parser.read_u16()?;
                ConstantPoolInfo::Methodref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_InterfaceMethodref => {
                let class_index = parser.read_u16()?;
                let name_and_type_index = parser.read_u16()?;
                ConstantPoolInfo::InterfaceMethodref {
                    class_index,
                    name_and_type_index,
                }
            }
            Self::TAG_NameAndType => {
                let name_index = parser.read_u16()?;
                let descriptor_index = parser.read_u16()?;
                ConstantPoolInfo::NameAndType {
                    name_index,
                    descriptor_index,
                }
            }
            Self::TAG_MethodHandle => {
                let reference_kind = parser.read_u8()?;
                let reference_index = parser.read_u16()?;
                ConstantPoolInfo::MethodHandle {
                    reference_kind,
                    reference_index,
                }
            }
            Self::TAG_MethodType => {
                let descriptor_index = parser.read_u16()?;
                ConstantPoolInfo::MethodType { descriptor_index }
            }
            Self::TAG_Dynamic => {
                let bootstrap_method_attr_index = parser.read_u16()?;
                let name_and_type_index = parser.read_u16()?;
                ConstantPoolInfo::Dynamic {
                    bootstrap_method_attr_index,
                    name_and_type_index,
                }
            }
            Self::TAG_InvokeDynamic => {
                let bootstrap_method_attr_index = parser.read_u16()?;
                let name_and_type_index = parser.read_u16()?;
                ConstantPoolInfo::InvokeDynamic {
                    bootstrap_method_attr_index,
                    name_and_type_index,
                }
            }
            Self::TAG_Module => {
                let name_index = parser.read_u16()?;
                ConstantPoolInfo::Module { name_index }
            }
            Self::TAG_Package => {
                let name_index = parser.read_u16()?;
                ConstantPoolInfo::Package { name_index }
            }
            _ => {
                return Err(Error::UnknownConstantPoolInfo { tag });
            }
        })
    }
}

/// An attribute in the class file format.
///
/// See [JVM Spec §4.7](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.7).
pub struct AttributeInfo {
    pub attribute_name_index: u16,
    pub attribute_length: u32,
    pub info: Box<[u8]>,
}

pub enum Error {
    UnexpectedEndOfFile,
    InvalidMagicNumber(u32),
    UnknownConstantPoolInfo { tag: u8 },
}

/// A parser for the class file format.
///
/// All methods in this parser are big-endian.
struct ClassFileParser<'a> {
    bytes: &'a [u8],
}

impl<'a> ClassFileParser<'a> {
    fn new(bytes: &'a [u8]) -> ClassFileParser<'a> {
        ClassFileParser { bytes }
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        if self.bytes.len() >= N {
            let (head, tail) = self.bytes.split_at(N);
            let arr: [u8; N] = head.try_into().map_err(|_| Error::UnexpectedEndOfFile)?;
            self.bytes = tail;
            Ok(arr)
        } else {
            Err(Error::UnexpectedEndOfFile)
        }
    }

    fn read_u8(&mut self) -> Result<u8, Error> {
        let [b] = self.read_array()?;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16, Error> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32, Error> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn read_u8_array(&mut self, size: usize) -> Result<Box<[u8]>, Error> {
        if self.bytes.len() >= size {
            let (head, tail) = self.bytes.split_at(size);
            self.bytes = tail;
            Ok(head.into())
        } else {
            Err(Error::UnexpectedEndOfFile)
        }
    }

    fn read_u16_array(&mut self, size: usize) -> Result<Box<[u16]>, Error> {
        let bytes_count = size * 2;
        if self.bytes.len() >= bytes_count {
            let (head, tail) = self.bytes.split_at(bytes_count);
            self.bytes = tail;
            Ok(head
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect())
        } else {
            Err(Error::UnexpectedEndOfFile)
        }
    }
}
