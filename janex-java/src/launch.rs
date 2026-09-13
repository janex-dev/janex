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
        self.prepare_with_resources(runtime, directory, limits, None)
    }

    /// Prepares a launch with an optional private classpath and module resource index.
    ///
    /// `resources` must be Host-generated launch data matching the embedded Java bootstrap.
    /// It requires bootstrap mode. The caller must retain the referenced private snapshot until
    /// Java exits. Java 9+ validates the indexed module graph in a separate process without
    /// executing application entry points or agents. Other behavior matches [`Self::prepare`].
    pub fn prepare_with_resources(
        &self,
        runtime: &JavaRuntime,
        directory: &Path,
        limits: Limits,
        resources: Option<&[u8]>,
    ) -> Result<Vec<OsString>> {
        if resources.is_some() && self.mode != LaunchMode::Bootstrap {
            return Err(invalid("resource loading requires bootstrap mode"));
        }
        if resources.is_some()
            && self
                .jvm_options
                .iter()
                .any(|option| option.starts_with("-Djava.system.class.loader="))
        {
            return Err(invalid(
                "resource loading requires the Janex system class loader; use direct mode for a custom loader",
            ));
        }
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
                resources,
                &self.jvm_options,
            )?)
        } else {
            None
        };
        let mut arguments = Vec::new();
        if resources.is_some()
            && runtime.feature >= 9
            && runtime
                .vm_name
                .as_deref()
                .is_some_and(|name| name.contains("OpenJDK") || name.contains("HotSpot"))
        {
            // Custom system loaders cannot use archived application classes. Keep the
            // expected HotSpot CDS notice out of application output; user options follow.
            arguments.push("-Xlog:cds=error".into());
        }
        let indexed_modules = resources.is_some() && runtime.feature >= 9;
        if indexed_modules {
            arguments.extend(indexed_module_options(&self.jvm_options)?);
        } else {
            arguments.extend(self.jvm_options.iter().map(OsString::from));
        }
        if resources.is_some() {
            arguments.push(
                "-Djava.system.class.loader=org.janex.bootstrap.loader.ResourceLoader".into(),
            );
        }
        if (self.entry_point.main_module.is_none() || resources.is_some())
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
        if let Some(module) = &self.entry_point.main_module
            && resources.is_none()
        {
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
        if indexed_modules {
            let system_modules = runtime.validate_indexed_modules(
                bridge.as_ref().expect("bootstrap resources require bridge"),
                &self.jvm_options,
            )?;
            if !system_modules.is_empty() {
                arguments.insert(
                    0,
                    format!("--add-modules={}", system_modules.join(",")).into(),
                );
            }
        }
        Ok(arguments)
    }
}

/// Retains native JVM options while deferring module access changes until the indexed layer exists.
pub(crate) fn indexed_module_options(options: &[String]) -> Result<Vec<OsString>> {
    let mut result = vec!["--add-exports=java.base/jdk.internal.module=ALL-UNNAMED".into()];
    let mut index = 0;
    while index < options.len() {
        let option = &options[index];
        let key = option.split('=').next().unwrap_or(option);
        if key == "--patch-module" {
            return Err(invalid("--patch-module requires direct launch mode"));
        }
        if matches!(
            key,
            "--add-modules"
                | "--add-reads"
                | "--add-exports"
                | "--add-opens"
                | "--enable-native-access"
        ) {
            let value = if let Some((_, value)) = option.split_once('=') {
                value
            } else {
                index += 1;
                options
                    .get(index)
                    .ok_or_else(|| invalid(format!("missing operand for {key}")))?
            };
            if key == "--enable-native-access" {
                result.push("--add-opens=java.base/java.lang=ALL-UNNAMED".into());
                if value.split(',').any(|name| name == "ALL-UNNAMED") {
                    result.push("--enable-native-access=ALL-UNNAMED".into());
                }
            }
        } else {
            result.push(option.into());
        }
        index += 1;
    }
    Ok(result)
}
