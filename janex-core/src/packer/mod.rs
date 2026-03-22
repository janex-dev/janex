// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

pub struct PackerConfigGroup {}

pub enum PackerConfigField {
    End,
    Condition,
    MainClass,
    MainModule,
    ModulePath,
    ClassPath,
    JvmProperties,
    AddReads,
    AddExports,
    AddOpens,
    EnableNativeAccess,
    ExtraJvmOptions,
    SubGroups,
}

impl PackerConfigField {
    pub const fn id(&self) -> u8 {
        match self {
            Self::End => 0,
            Self::Condition => 1,
            Self::MainClass => 2,
            Self::MainModule => 3,
            Self::ModulePath => 4,
            Self::ClassPath => 5,
            Self::JvmProperties => 6,
            Self::AddReads => 7,
            Self::AddExports => 8,
            Self::AddOpens => 9,
            Self::EnableNativeAccess => 10,
            Self::ExtraJvmOptions => 11,
            Self::SubGroups => 12,
        }
    }
}
