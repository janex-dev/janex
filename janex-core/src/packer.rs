// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

pub struct PackerConfigGroup {
    pub main_class: String,
    pub main_module: String,
    pub condition: String,
    pub extra_jvm_options: Vec<String>,
}

impl PackerConfigGroup {
    pub const MAGIC_NUMBER: u32 = 0x50524743;
}

pub enum PackerConfigField {
    End,
    Condition,
    MainClass,
    MainModule,
    ModulePath,
    ClassPath,
    JvmOptions,
    SubGroups,
}

impl PackerConfigField {
    pub const END: u8 = 0;
    pub const CONDITION: u8 = 1;
    pub const MAIN_CLASS: u8 = 2;
    pub const MAIN_MODULE: u8 = 3;
    pub const MODULE_PATH: u8 = 4;
    pub const CLASS_PATH: u8 = 5;
    pub const JVM_OPTIONS: u8 = 6;
    pub const SUB_GROUPS: u8 = 0xff;

    pub const fn id(&self) -> u8 {
        match self {
            PackerConfigField::End => Self::END,
            PackerConfigField::Condition => Self::CONDITION,
            PackerConfigField::MainClass => Self::MAIN_CLASS,
            PackerConfigField::MainModule => Self::MAIN_MODULE,
            PackerConfigField::ModulePath => Self::MODULE_PATH,
            PackerConfigField::ClassPath => Self::CLASS_PATH,
            PackerConfigField::JvmOptions => Self::JVM_OPTIONS,
            PackerConfigField::SubGroups => Self::SUB_GROUPS,
        }
    }
}
