// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringPool {
    strings: Vec<String>,
    indices: BTreeMap<String, u64>,
}

impl StringPool {
    pub fn new(strings: Vec<String>) -> Result<Self, Error> {
        let mut pool = Self {
            strings: Vec::with_capacity(strings.len()),
            indices: BTreeMap::new(),
        };

        for string in strings {
            pool.push_existing(string)?;
        }

        if pool.strings.is_empty() {
            return Err(Error::InvalidValue(
                "string pool must contain at least one string",
            ));
        }

        if !pool.strings[0].is_empty() {
            return Err(Error::InvalidValue(
                "string pool index 0 must be an empty string",
            ));
        }

        Ok(pool)
    }

    pub fn with_empty_root() -> Self {
        let mut indices = BTreeMap::new();
        indices.insert(String::new(), 0);
        Self {
            strings: vec![String::new()],
            indices,
        }
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    pub fn get(&self, index: u64) -> Option<&str> {
        self.strings.get(index as usize).map(String::as_str)
    }

    pub fn push(&mut self, value: impl Into<String>) -> u64 {
        let value = value.into();
        if let Some(index) = self.indices.get(&value) {
            return *index;
        }

        let index = self.strings.len() as u64;
        self.indices.insert(value.clone(), index);
        self.strings.push(value);
        index
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &str> {
        self.strings.iter().map(String::as_str)
    }

    fn push_existing(&mut self, value: String) -> Result<(), Error> {
        if self.indices.contains_key(&value) {
            return Err(Error::InvalidValue(
                "string pool contains duplicated strings",
            ));
        }

        let index = self.strings.len() as u64;
        self.indices.insert(value.clone(), index);
        self.strings.push(value);
        Ok(())
    }
}

impl Default for StringPool {
    fn default() -> Self {
        Self::with_empty_root()
    }
}
