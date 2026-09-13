// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Local Java executable discovery and runtime property probes.

use crate::{Result, error::invalid};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Explicit runtime overrides; at most one field may be supplied.
#[derive(Clone, Debug, Default)]
pub struct JavaOptions {
    /// Java executable path, or a bare executable name resolved on PATH.
    pub java: Option<PathBuf>,
    /// Java home whose `bin/java` or `bin/java.exe` should be used.
    pub java_home: Option<PathBuf>,
}

impl JavaOptions {
    /// Returns whether an explicit override prevents falling back to another runtime.
    pub fn is_explicit(&self) -> bool {
        self.java.is_some() || self.java_home.is_some()
    }
}

/// Properties of a successfully probed Java 8-or-later executable.
#[derive(Clone, Debug)]
pub struct JavaRuntime {
    /// Absolute executable path used for both discovery and launching.
    pub executable: PathBuf,
    /// Runtime-reported Java home, which can differ from the executable's parent layout.
    pub home: PathBuf,
    /// Feature number reported by the runtime version, such as 8, 17, or 25.
    pub feature: u32,
    /// Exact `java.version` property for diagnostics.
    pub version_text: String,
    /// Exact `java.vendor` property for condition matching.
    pub vendor: String,
    /// Normalized `os.arch` of this JVM, independent of the launcher and native system.
    pub architecture: String,
    /// Runtime-reported VM name, when included in the property probe.
    pub vm_name: Option<String>,
    /// System module names and optional descriptor versions; empty for Java 8.
    pub modules: BTreeMap<String, Option<String>>,
}

impl JavaRuntime {
    /// Validates an indexed bootstrap's module graph without application main methods or agents.
    pub(crate) fn validate_indexed_modules(
        &self,
        bridge: &Path,
        options: &[String],
    ) -> Result<Vec<String>> {
        let mut resolution = Vec::new();
        let mut index = 0;
        while index < options.len() {
            let key = options[index].split('=').next().unwrap();
            if matches!(
                key,
                "--limit-modules" | "--upgrade-module-path" | "--enable-native-access"
            ) {
                resolution.push(options[index].clone());
                if !options[index].contains('=') {
                    index += 1;
                    resolution.push(
                        options
                            .get(index)
                            .ok_or_else(|| invalid("missing module option operand"))?
                            .clone(),
                    );
                }
            }
            index += 1;
        }
        let mut args = crate::launch::indexed_module_options(&resolution)?;
        args.push("--add-modules=ALL-SYSTEM".into());
        args.push("-cp".into());
        args.push(bridge.as_os_str().into());
        args.push("org.glavo.janex.bootstrap.loader.ModuleSupport".into());
        let output = Command::new(&self.executable)
            .args(&args)
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(invalid(format!(
                "Java module validation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let text =
            String::from_utf8(output.stdout).map_err(|_| invalid("invalid module probe output"))?;
        let names: Vec<String> = text.lines().map(str::to_owned).collect();
        if names.iter().any(|name| {
            name.is_empty() || name.contains([',', '=', '\0']) || !self.modules.contains_key(name)
        }) {
            return Err(invalid("invalid module probe output"));
        }
        Ok(names)
    }
    /// Probes one executable's properties and, on Java 9 or later, system modules.
    ///
    /// Probes start child processes without a shell, capture their output, and do not
    /// change the environment of the calling process. UTF-8 stream properties apply
    /// only to probes. An unsuccessful or malformed probe is an error.
    pub fn probe(executable: &Path) -> Result<Self> {
        let executable = executable.canonicalize()?;
        let properties = probe_output(
            &executable,
            &["-XshowSettings:properties", "-version"],
            true,
        )?;
        let (home, version_text, vendor) = settings(&properties)?;
        let architectures: Vec<_> = properties
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("os.arch = "))
            .collect();
        if architectures.len() != 1 || architectures[0].is_empty() {
            return Err(invalid("missing or duplicate Java os.arch property"));
        }
        let architecture = janex_platform::normalize_architecture(architectures[0]).to_owned();
        let vm_name = properties.lines().find_map(|line| {
            line.trim_start()
                .strip_prefix("java.vm.name = ")
                .map(str::to_owned)
        });
        let feature = feature_version(&version_text)?;
        let modules = if feature >= 9 {
            module_list(&probe_output(&executable, &["--list-modules"], false)?)?
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            executable,
            home: home.into(),
            feature,
            version_text,
            vendor,
            architecture,
            vm_name,
            modules,
        })
    }

    /// Validates local module-path entries and returns all observable module names and versions.
    ///
    /// Uses this runtime's module finder, including filename-derived automatic module names.
    /// Java 8 accepts only an empty path. Probes do not run application main methods or agents.
    pub fn validate_module_path(
        &self,
        paths: &[PathBuf],
    ) -> Result<BTreeMap<String, Option<String>>> {
        if paths.is_empty() {
            return Ok(self.modules.clone());
        }
        if self.feature < 9 {
            return Err(invalid("the module path requires Java 9 or later"));
        }
        let path = join_path(paths)?;
        let arguments = [
            OsStr::new("--module-path"),
            &path,
            OsStr::new("--validate-modules"),
        ];
        probe_output(&self.executable, &arguments, false)?;
        let arguments = [
            OsStr::new("--module-path"),
            &path,
            OsStr::new("--add-modules"),
            OsStr::new("ALL-MODULE-PATH"),
            OsStr::new("-version"),
        ];
        probe_output(&self.executable, &arguments, true)?;
        let arguments = [
            OsStr::new("--module-path"),
            &path,
            OsStr::new("--list-modules"),
        ];
        let output = probe_output(&self.executable, &arguments, false)?;
        let descriptors = output
            .lines()
            .map(|line| {
                line.rsplit_once(" file:")
                    .map_or(line, |(descriptor, _)| descriptor)
            })
            .collect::<Vec<_>>()
            .join("\n");
        module_list(&descriptors)
    }
}

/// Joins explicit Java path entries, rejecting separators that Java cannot escape.
///
/// Windows verbatim disk and UNC prefixes are converted to ordinary absolute paths for Java.
pub fn join_path(paths: &[PathBuf]) -> Result<OsString> {
    let separator = if cfg!(windows) { b';' } else { b':' };
    if paths
        .iter()
        .any(|path| path.as_os_str().as_encoded_bytes().contains(&separator))
    {
        return Err(invalid(
            "Java path entry contains the platform path separator",
        ));
    }
    std::env::join_paths(paths.iter().map(|path| java_path(path)))
        .map_err(|error| invalid(format!("invalid Java path: {error}")))
}

/// Converts Windows filesystem prefixes that older Java runtimes reject.
fn java_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();
        if let Some(Component::Prefix(prefix)) = components.next() {
            let mut result = match prefix.kind() {
                Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:", char::from(drive))),
                Prefix::VerbatimUNC(server, share) => {
                    let mut unc = OsString::from("\\\\");
                    unc.push(server);
                    unc.push("\\");
                    unc.push(share);
                    PathBuf::from(unc)
                }
                _ => return path.to_owned(),
            };
            result.push(components.as_path());
            return result;
        }
    }
    path.to_owned()
}

/// Returns executable candidates in explicit-override, JAVA_HOME, then PATH order.
///
/// An explicit override returns exactly one candidate. A missing executable under
/// JAVA_HOME is retained so the caller can report its failed probe before trying PATH.
/// PATH contributes existing regular files, with canonical duplicates removed.
/// This function neither starts Java nor tests application compatibility.
pub fn candidates(options: &JavaOptions) -> Result<Vec<PathBuf>> {
    candidates_from(
        options,
        std::env::var_os("JAVA_HOME").as_deref(),
        std::env::var_os("PATH").as_deref(),
    )
}

/// Probes discovered runtimes, preferring the native system architecture for automatic selection.
///
/// Explicit executable or home overrides are never replaced. Within each architecture group,
/// JAVA_HOME and PATH order is retained. Other runnable architectures remain fallback candidates.
/// Discovery failure leaves the original order intact. Failed probes are skipped unless no
/// runtime succeeds, in which case their diagnostics are returned together.
pub fn runtimes(options: &JavaOptions) -> Result<Vec<JavaRuntime>> {
    let mut runtimes = Vec::new();
    let mut failures = Vec::new();
    for path in candidates(options)? {
        match JavaRuntime::probe(&path) {
            Ok(runtime) => runtimes.push(runtime),
            Err(error) if options.is_explicit() => return Err(error),
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    if runtimes.is_empty() {
        return Err(invalid(if failures.is_empty() {
            "no Java runtime found; specify --java or --java-home".into()
        } else {
            format!("no usable Java runtime:\n{}", failures.join("\n"))
        }));
    }
    if !options.is_explicit()
        && let Ok(native) = janex_platform::native_architecture()
    {
        prefer_native(&mut runtimes, &native);
    }
    Ok(runtimes)
}

/// Stably ranks runtimes without rejecting working emulated architectures.
fn prefer_native(runtimes: &mut [JavaRuntime], native: &str) {
    runtimes.sort_by_key(|runtime| runtime.architecture != native);
}

/// Resolves candidates using supplied environment values, without mutating process globals.
fn candidates_from(
    options: &JavaOptions,
    java_home: Option<&OsStr>,
    path: Option<&OsStr>,
) -> Result<Vec<PathBuf>> {
    if options.java.is_some() && options.java_home.is_some() {
        return Err(invalid("--java and --java-home are mutually exclusive"));
    }
    if let Some(home) = &options.java_home {
        return Ok(vec![absolute(&home.join("bin").join(java_name()))?]);
    }
    if let Some(java) = &options.java {
        if java.is_absolute() || java.components().count() > 1 {
            return Ok(vec![absolute(java)?]);
        }
        if let Some(path) = path {
            for directory in std::env::split_paths(path) {
                let candidate = directory.join(java);
                if candidate.is_file() {
                    return Ok(vec![absolute(&candidate)?]);
                }
                #[cfg(windows)]
                if java.extension().is_none() {
                    let candidate = candidate.with_extension("exe");
                    if candidate.is_file() {
                        return Ok(vec![absolute(&candidate)?]);
                    }
                }
            }
        }
        return Err(invalid(format!(
            "Java executable not found on PATH: {}",
            java.display()
        )));
    }
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(home) = java_home.filter(|home| !home.is_empty()) {
        let candidate = absolute(&Path::new(home).join("bin").join(java_name()))?;
        seen.insert(
            candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone()),
        );
        result.push(candidate);
    }
    if let Some(path) = path {
        for directory in std::env::split_paths(path) {
            let candidate = directory.join(java_name());
            if candidate.is_file() {
                let candidate = absolute(&candidate)?;
                if seen.insert(
                    candidate
                        .canonicalize()
                        .unwrap_or_else(|_| candidate.clone()),
                ) {
                    result.push(candidate);
                }
            }
        }
    }
    Ok(result)
}

/// Starts a Java probe with captured UTF-8 output and no inherited standard input.
fn probe_output(
    executable: &Path,
    arguments: &[impl AsRef<OsStr>],
    stderr: bool,
) -> Result<String> {
    let mut command = Command::new(executable);
    command
        .args([
            "-Dfile.encoding=UTF-8",
            "-Dstdout.encoding=UTF-8",
            "-Dstderr.encoding=UTF-8",
            "-Dsun.stdout.encoding=UTF-8",
            "-Dsun.stderr.encoding=UTF-8",
        ])
        .args(arguments)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(invalid(format!(
            "Java probe failed for {}: {}\n{}",
            executable.display(),
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(if stderr { output.stderr } else { output.stdout })
        .map_err(|_| invalid("Java probe output is not UTF-8"))
}

/// Extracts the feature number from a reported Java version; full version policy belongs to the caller.
fn feature_version(version: &str) -> Result<u32> {
    let version = version.strip_prefix("1.").unwrap_or(version);
    let feature = version.split(['.', '_', '-', '+']).next().unwrap_or("");
    if feature.is_empty() || !feature.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("invalid Java feature version"));
    }
    feature
        .parse::<u32>()
        .ok()
        .filter(|feature| *feature >= 8)
        .ok_or_else(|| invalid("Java 8 or later is required"))
}

/// Extracts the runtime-reported home, version, and vendor from property settings.
fn settings(output: &str) -> Result<(String, String, String)> {
    let mut properties = BTreeMap::new();
    for line in output.lines() {
        if let Some((name, value)) = line.trim_start().split_once(" = ")
            && matches!(name, "java.home" | "java.version" | "java.vendor")
            && (value.is_empty() || properties.insert(name, value).is_some())
        {
            return Err(invalid("missing or duplicate Java probe property"));
        }
    }
    let required = |key| {
        properties
            .get(key)
            .map(|value| (*value).to_owned())
            .ok_or_else(|| invalid(format!("Java probe did not report {key}")))
    };
    Ok((
        required("java.home")?,
        required("java.version")?,
        required("java.vendor")?,
    ))
}

/// Parses system module names and exact optional module versions.
fn module_list(output: &str) -> Result<BTreeMap<String, Option<String>>> {
    let mut modules = BTreeMap::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let (name, version) = line
            .split_once('@')
            .map_or((line, None), |(name, version)| (name, Some(version)));
        if name.is_empty()
            || name.chars().any(char::is_whitespace)
            || version == Some("")
            || modules
                .insert(name.into(), version.map(str::to_owned))
                .is_some()
        {
            return Err(invalid("invalid or duplicate Java system module"));
        }
    }
    if !modules.contains_key("java.base") {
        return Err(invalid("Java system module list omits java.base"));
    }
    Ok(modules)
}

/// Returns the platform's Java executable filename.
fn java_name() -> &'static str {
    if cfg!(windows) { "java.exe" } else { "java" }
}

/// Makes a path absolute without requiring its target to exist.
fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.into()
    } else {
        std::env::current_dir()?.join(path)
    })
}

#[cfg(test)]
mod tests {
    //! Pure settings and candidate-order tests without global environment mutation.
    use super::*;
    use std::fs;

    #[test]
    fn native_preference_keeps_foreign_runtimes_as_stable_fallbacks() {
        let runtime = |name: &str, architecture: &str| JavaRuntime {
            executable: name.into(),
            home: name.into(),
            feature: 25,
            version_text: "25".into(),
            vendor: "Test".into(),
            architecture: architecture.into(),
            vm_name: None,
            modules: BTreeMap::new(),
        };
        let mut values = vec![
            runtime("x86", "x86"),
            runtime("x64", "x86-64"),
            runtime("arm", "aarch64"),
            runtime("arm2", "aarch64"),
        ];
        prefer_native(&mut values, "aarch64");
        assert_eq!(
            values
                .iter()
                .map(|value| value.executable.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["arm", "arm2", "x86", "x64"]
        );
        prefer_native(&mut values, "x86-64");
        assert_eq!(values[0].executable, Path::new("x64"));
    }

    #[cfg(windows)]
    #[test]
    fn java_paths_accept_canonical_disk_and_unc_paths() {
        assert_eq!(
            join_path(&[
                PathBuf::from(r"\\?\C:\packages\app.jar"),
                PathBuf::from(r"\\?\UNC\server\share\app.jar")
            ])
            .unwrap(),
            OsString::from(r"C:\packages\app.jar;\\server\share\app.jar")
        );
        assert!(join_path(&[PathBuf::from(r"C:\bad;path.jar")]).is_err());
    }

    #[test]
    fn parses_java_8_aliases_unicode_properties_and_module_versions() {
        let (home, version, vendor) = settings("Property settings:\n    java.home = /runtime/\u{4e2d}\n    java.version = 1.8.0_402\n    java.vendor = Example Vendor\n").unwrap();
        assert_eq!(home, "/runtime/\u{4e2d}");
        assert_eq!(vendor, "Example Vendor");
        assert_eq!(feature_version(&version).unwrap(), 8);
        let modules = module_list("java.base@25\njava.logging@25\nexample.module\n").unwrap();
        assert_eq!(modules["java.base"].as_deref(), Some("25"));
        assert_eq!(modules["example.module"], None);
        assert!(module_list("java.base@25\njava.base@25\n").is_err());
        assert!(module_list("java.logging@25\n").is_err());
        assert!(settings("java.version = 25\n").is_err());
        assert!(
            settings("java.home = /jdk\njava.home = /other\njava.version = 25\njava.vendor = V\n")
                .is_err()
        );
    }

    #[test]
    fn candidate_order_respects_overrides_and_deduplicates_paths() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let first = home.join("bin");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir(&second).unwrap();
        fs::write(first.join(java_name()), b"").unwrap();
        fs::write(second.join(java_name()), b"").unwrap();
        let path = std::env::join_paths([&second, &first, &second]).unwrap();
        let result =
            candidates_from(&JavaOptions::default(), Some(home.as_os_str()), Some(&path)).unwrap();
        assert_eq!(result, [first.join(java_name()), second.join(java_name())]);
        let override_options = JavaOptions {
            java: Some(second.join(java_name())),
            java_home: None,
        };
        assert!(override_options.is_explicit());
        assert_eq!(
            candidates_from(&override_options, Some(home.as_os_str()), Some(&path)).unwrap(),
            [second.join(java_name())]
        );
        let missing = JavaOptions {
            java: None,
            java_home: Some(temp.path().join("missing")),
        };
        assert_eq!(
            candidates_from(&missing, None, Some(&path)).unwrap().len(),
            1
        );
        let conflict = JavaOptions {
            java: override_options.java,
            java_home: missing.java_home,
        };
        assert!(candidates_from(&conflict, None, Some(&path)).is_err());
    }
}
