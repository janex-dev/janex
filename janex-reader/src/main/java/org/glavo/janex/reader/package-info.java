// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

/// Reads Janex containers and exposes validated metadata and immutable resource descriptions.
///
/// Stored resource descriptions borrow an unchanged snapshot; closing a reader closes its channel,
/// while returned metadata and resource descriptions retain their documented lifetimes.
package org.glavo.janex.reader;

