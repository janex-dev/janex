// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

pub struct PackerConfigGroup {
    pub jvm_properties: Vec<String>,
    pub add_reads: Vec<String>,
    pub add_exports: Vec<String>,
    pub add_opens: Vec<String>,
    pub enable_native_access: Vec<String>,
    pub extra_jvm_options: Vec<String>,
}

pub enum PackerConfigField {
    End,
    Condition,
    MainClass,
    MainModule,
    ModulePath,
    ClassPath,
    JvmProperties,
    JvmOptions,
    SubGroups,
}

impl PackerConfigField {
    /// Magic number for each config field.
    ///
    /// This is used to identify the start of a config field in the packed file.
    pub const MAGIC_NUMBER: u32 = 0x00505247;

    pub const END: u8 = 0;
    pub const CONDITION: u8 = 1;
    pub const MAIN_CLASS: u8 = 2;
    pub const MAIN_MODULE: u8 = 3;
    pub const MODULE_PATH: u8 = 4;
    pub const CLASS_PATH: u8 = 5;
    pub const JVM_PROPERTIES: u8 = 6;
    pub const JVM_OPTIONS: u8 = 7;
    pub const SUB_GROUPS: u8 = 127;

    pub const fn id(&self) -> u8 {
        match self {
            PackerConfigField::End => Self::END,
            PackerConfigField::Condition => Self::CONDITION,
            PackerConfigField::MainClass => Self::MAIN_CLASS,
            PackerConfigField::MainModule => Self::MAIN_MODULE,
            PackerConfigField::ModulePath => Self::MODULE_PATH,
            PackerConfigField::ClassPath => Self::CLASS_PATH,
            PackerConfigField::JvmProperties => Self::JVM_PROPERTIES,
            PackerConfigField::JvmOptions => Self::JVM_OPTIONS,
            PackerConfigField::SubGroups => Self::SUB_GROUPS,
        }
    }
}
