// Copyright (c) 2025 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Local packaging, signing, authentication, and Java launch services for Janex containers.

mod adapters;
pub mod app;
pub mod authentication;
mod bootstrap;
pub mod dependency;
mod error;
pub mod import;
pub mod integration;
pub mod materialize;
pub mod maven;
mod modules;
pub mod native_launcher;
pub mod pack;
mod persistence;
mod roots;
pub mod run;
pub mod sdk;
pub mod self_manage;

pub use error::{Error, Result};
