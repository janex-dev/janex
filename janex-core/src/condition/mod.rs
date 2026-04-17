// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Runtime types and CEL evaluation helpers for Janex conditions.

mod java;
pub mod platform;

use crate::error::Error;
use crate::janex::{ConfigField, ConfigGroup};
use cel_interpreter::{Context as CelContext, Program, Value};

pub use java::{Java, JavaVersion};
pub use platform::{Cpu, OperatingSystem, OperatingSystemVersion, Platform};

/// The result of evaluating a CEL condition expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionValue {
    /// The expression evaluated to a boolean applicability result.
    Bool(bool),
    /// The expression evaluated to an integer priority.
    Int(i64),
}

/// The result of evaluating a root `ConfigGroup` condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootConditionResult {
    /// The candidate Java runtime is not eligible.
    Rejected,
    /// The candidate Java runtime is eligible with the given priority.
    Accepted {
        /// The priority score used to rank candidate Java runtimes.
        priority: i64,
    },
}

impl RootConditionResult {
    /// Returns whether the root condition accepted the current environment.
    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    /// Returns the candidate priority when the root condition accepted the environment.
    pub const fn priority(self) -> Option<i64> {
        match self {
            Self::Rejected => None,
            Self::Accepted { priority } => Some(priority),
        }
    }
}

/// The runtime values exposed to CEL condition expressions as `java` and `platform`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionEnvironment {
    /// Information about the candidate Java runtime.
    pub java: Java,
    /// Information about the current host platform.
    pub platform: Platform,
}

impl ConditionEnvironment {
    /// Creates a condition environment from explicit Java and platform descriptors.
    pub fn new(java: Java, platform: Platform) -> Self {
        Self { java, platform }
    }

    /// Compiles and evaluates a condition expression once.
    pub fn evaluate_condition(&self, source: &str) -> Result<ConditionValue, Error> {
        ConditionProgram::compile(source)?.evaluate(self)
    }

    /// Compiles and evaluates a root-group condition once.
    pub fn evaluate_root_condition(&self, source: &str) -> Result<RootConditionResult, Error> {
        ConditionProgram::compile(source)?.evaluate_root(self)
    }

    /// Compiles and evaluates a subgroup condition once.
    pub fn evaluate_group_condition(&self, source: &str) -> Result<bool, Error> {
        ConditionProgram::compile(source)?.evaluate_group(self)
    }
}

/// A compiled CEL program that can be reused across multiple condition evaluations.
#[derive(Debug)]
pub struct ConditionProgram {
    source: String,
    program: Program,
}

impl ConditionProgram {
    /// Compiles a CEL condition expression.
    pub fn compile(source: impl Into<String>) -> Result<Self, Error> {
        let source = source.into();
        let program =
            Program::compile(&source).map_err(|error| Error::ConditionParse(error.to_string()))?;
        Ok(Self { source, program })
    }

    /// Returns the original CEL source string.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Evaluates the compiled CEL expression and returns its raw Janex condition value.
    pub fn evaluate(&self, environment: &ConditionEnvironment) -> Result<ConditionValue, Error> {
        let mut context = CelContext::default();
        context
            .add_variable("java", &environment.java)
            .map_err(|error| {
                Error::ConditionExecution(format!(
                    "failed to serialize variable 'java' for condition '{}': {error}",
                    self.source
                ))
            })?;
        context
            .add_variable("platform", &environment.platform)
            .map_err(|error| {
                Error::ConditionExecution(format!(
                    "failed to serialize variable 'platform' for condition '{}': {error}",
                    self.source
                ))
            })?;

        let value = self.program.execute(&context).map_err(|error| {
            Error::ConditionExecution(format!(
                "failed to execute condition '{}': {error}",
                self.source
            ))
        })?;
        condition_value_from_cel(value)
    }

    /// Evaluates the compiled CEL expression using root-group semantics.
    pub fn evaluate_root(
        &self,
        environment: &ConditionEnvironment,
    ) -> Result<RootConditionResult, Error> {
        match self.evaluate(environment)? {
            ConditionValue::Bool(false) => Ok(RootConditionResult::Rejected),
            ConditionValue::Bool(true) => Ok(RootConditionResult::Accepted { priority: 0 }),
            ConditionValue::Int(priority) => Ok(RootConditionResult::Accepted { priority }),
        }
    }

    /// Evaluates the compiled CEL expression using subgroup semantics.
    pub fn evaluate_group(&self, environment: &ConditionEnvironment) -> Result<bool, Error> {
        match self.evaluate(environment)? {
            ConditionValue::Bool(value) => Ok(value),
            ConditionValue::Int(_) => Err(Error::ConditionExecution(format!(
                "condition '{}' evaluated to int, but non-root config groups require bool",
                self.source
            ))),
        }
    }
}

impl ConfigGroup {
    /// Returns the optional CEL condition guarding this configuration group.
    pub fn condition(&self) -> Option<&str> {
        self.fields.iter().find_map(|field| match field {
            ConfigField::Condition(condition) => Some(condition.as_str()),
            _ => None,
        })
    }

    /// Evaluates this configuration group as the root group.
    pub fn evaluate_root_condition(
        &self,
        environment: &ConditionEnvironment,
    ) -> Result<RootConditionResult, Error> {
        match self.condition() {
            Some(condition) => environment.evaluate_root_condition(condition),
            None => Ok(RootConditionResult::Accepted { priority: 0 }),
        }
    }

    /// Evaluates this configuration group as a subgroup.
    pub fn evaluate_group_condition(
        &self,
        environment: &ConditionEnvironment,
    ) -> Result<bool, Error> {
        match self.condition() {
            Some(condition) => environment.evaluate_group_condition(condition),
            None => Ok(true),
        }
    }
}

fn condition_value_from_cel(value: Value) -> Result<ConditionValue, Error> {
    match value {
        Value::Bool(value) => Ok(ConditionValue::Bool(value)),
        Value::Int(value) => Ok(ConditionValue::Int(value)),
        other => Err(Error::ConditionExecution(format!(
            "condition must evaluate to bool or int, got {}",
            other.type_of()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_environment() -> ConditionEnvironment {
        ConditionEnvironment::new(
            Java::new(
                JavaVersion::parse("21.0.3+9").unwrap(),
                "Eclipse Adoptium",
                OperatingSystem::new("windows", OperatingSystemVersion::parse("10.0.22631")),
                "x86_64",
            ),
            Platform::new(
                OperatingSystem::new("windows", OperatingSystemVersion::parse("10.0.22631")),
                Cpu::new("x86_64"),
            ),
        )
    }

    #[test]
    fn java_version_parse_extracts_documented_components() -> Result<(), Error> {
        let version = JavaVersion::parse("21.0.3-ea+9-LTS")?;
        assert_eq!(version.full, "21.0.3-ea+9-LTS");
        assert_eq!(version.feature, 21);
        assert_eq!(version.interim, 0);
        assert_eq!(version.update, 3);
        assert_eq!(version.patch, 0);
        assert_eq!(version.pre, "ea");
        assert_eq!(version.build, 9);
        assert_eq!(version.optional, "LTS");
        Ok(())
    }

    #[test]
    fn root_conditions_accept_integer_priorities() -> Result<(), Error> {
        let program = ConditionProgram::compile("int(java.version.feature) - 20")?;
        assert_eq!(
            program.evaluate_root(&sample_environment())?,
            RootConditionResult::Accepted { priority: 1 }
        );
        Ok(())
    }

    #[test]
    fn subgroup_conditions_require_boolean_results() {
        let program = ConditionProgram::compile("int(java.version.feature) - 20").unwrap();
        let error = program.evaluate_group(&sample_environment()).unwrap_err();
        assert!(matches!(error, Error::ConditionExecution(_)));
    }

    #[test]
    fn config_group_uses_condition_field() -> Result<(), Error> {
        let group = ConfigGroup {
            fields: vec![ConfigField::Condition(
                "java.version.feature >= 21 && platform.os.name == 'windows'".to_string(),
            )],
        };
        assert!(group.evaluate_group_condition(&sample_environment())?);
        Ok(())
    }

    #[test]
    fn platform_current_exposes_normalized_fields() {
        let platform = Platform::current();
        assert!(!platform.os.name.is_empty());
        assert!(!platform.cpu.arch.is_empty());
    }
}
