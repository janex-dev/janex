// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Container-independent Java invocation inputs and argument construction.

use crate::{Limits, Result, bootstrap, error::invalid, runtime::JavaRuntime};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

/// Selects how Java reaches the application entry point.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LaunchMode {
    /// Uses a Java 8-compatible entry layer to preserve Unicode program arguments.
    ///
    /// Unix arguments must be Unicode; Windows UTF-16 code units are retained verbatim.
    #[default]
    Bootstrap,
    /// Invokes the runtime's native launcher with the application entry point directly.
    ///
    /// Argument conversion follows that launcher's behavior, including Windows code-page limits.
    /// Preparation rejects NUL in native process arguments.
    Direct,
}

/// A Java class or module entry point supplied by the caller.
#[derive(Clone, Debug, Default)]
pub struct EntryPoint {
    /// Binary class name; may be omitted when the module descriptor supplies it.
    pub main_class: Option<String>,
    /// Main module name; absent for a classpath application.
    pub main_module: Option<String>,
}

/// Evaluated Java invocation inputs with local class and module paths.
#[derive(Clone, Debug)]
pub struct LaunchRequest {
    /// Application entry point.
    pub entry_point: EntryPoint,
    /// Explicit entry strategy; independent of runtime discovery.
    pub mode: LaunchMode,
    /// Ordered JVM options, without shell quoting.
    pub jvm_options: Vec<String>,
    /// Ordered local classpath entries.
    pub class_path: Vec<PathBuf>,
    /// Ordered local module-path entries.
    pub module_path: Vec<PathBuf>,
    /// Complete native agent arguments, already resolved by the caller.
    pub agents: Vec<OsString>,
    /// Preset and user program arguments in their final order.
    pub arguments: Vec<OsString>,
}

impl LaunchRequest {
    /// Builds native arguments and writes required private resources into an existing directory.
    ///
    /// The caller must supply a private directory and retain it until the child exits.
    /// Failure may leave partial resources there. This does not start Java or authenticate input.
    /// Module launching requires Java 9 or later. Bootstrap argument encoding is bounded by `limits`.
    pub fn prepare(
        &self,
        runtime: &JavaRuntime,
        directory: &Path,
        limits: Limits,
    ) -> Result<Vec<OsString>> {
        limits.elements(self.arguments.len() as u64)?;
        if runtime.feature < 9
            && (self.entry_point.main_module.is_some() || !self.module_path.is_empty())
        {
            return Err(invalid("module launching requires Java 9 or later"));
        }
        if self.entry_point.main_module.is_none() && self.entry_point.main_class.is_none() {
            return Err(invalid("classpath launch requires a main class"));
        }
        let mut class_path = self.class_path.clone();
        let module_path = &self.module_path;
        let agents = self.agents.iter().cloned();
        let program_arguments = &self.arguments;
        let bridge = if self.mode == LaunchMode::Bootstrap {
            Some(bootstrap::write(
                directory,
                &self.entry_point,
                program_arguments,
                runtime.feature >= 25
                    || (runtime.feature >= 21
                        && self
                            .jvm_options
                            .iter()
                            .any(|option| option == "--enable-preview")),
                limits,
            )?)
        } else {
            None
        };
        let mut arguments = Vec::new();
        if runtime.feature >= 9 {
            arguments.push("--disable-@files".into());
        }
        arguments.extend(self.jvm_options.iter().map(OsString::from));
        if self.entry_point.main_module.is_none()
            && let Some(bridge) = &bridge
        {
            class_path.insert(0, bridge.clone());
        }
        if class_path.is_empty() {
            let empty = directory.join("empty-classpath");
            fs::create_dir(&empty)?;
            class_path.push(empty);
        }
        arguments.push("-cp".into());
        arguments.push(crate::runtime::join_path(&class_path)?);
        if !module_path.is_empty() {
            arguments.push("--module-path".into());
            arguments.push(crate::runtime::join_path(module_path)?);
        }
        arguments.extend(agents);
        if let Some(module) = &self.entry_point.main_module {
            let entry = if let Some(bridge) = &bridge {
                arguments.push("--patch-module".into());
                let mut patch = OsString::from(format!("{module}="));
                patch.push(bridge);
                arguments.push(patch);
                format!("{module}/{}", bootstrap::MAIN_CLASS)
            } else {
                self.entry_point
                    .main_class
                    .as_ref()
                    .map_or_else(|| module.clone(), |class| format!("{module}/{class}"))
            };
            arguments.push("--module".into());
            arguments.push(entry.into());
        } else {
            arguments.push(
                if bridge.is_some() {
                    bootstrap::MAIN_CLASS
                } else {
                    self.entry_point
                        .main_class
                        .as_deref()
                        .ok_or_else(|| invalid("classpath launch requires a main class"))?
                }
                .into(),
            );
        }
        if bridge.is_none() {
            arguments.extend(program_arguments.iter().cloned());
        }
        if arguments
            .iter()
            .any(|argument| argument.as_encoded_bytes().contains(&0))
        {
            return Err(invalid("Java process arguments must not contain NUL"));
        }
        Ok(arguments)
    }
}
