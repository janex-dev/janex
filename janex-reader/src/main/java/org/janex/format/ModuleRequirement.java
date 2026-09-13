// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.*;
import java.net.URI;
import java.net.URISyntaxException;

import static org.janex.format.Input.*;

/// Parses canonical Janex module requirements without acquiring a provider.
final class ModuleRequirement {
    /// Exact descriptor name.
    final String name;
    /// Exact descriptor version, or empty when unconstrained.
    final String version;

    /// Retains a decoded module requirement.
    private ModuleRequirement(String name, String version) {
        this.name = name;
        this.version = version;
    }

    /// Returns a Janex requirement, or null for another absolute URI; validates Janex path placement.
    static ModuleRequirement parse(String value, boolean module) throws IOException {
        URI uri;
        try {
            uri = new URI(value);
        } catch (URISyntaxException failure) {
            Invalid invalid = new Invalid("Invalid external resource URI");
            invalid.initCause(failure);
            throw invalid;
        }
        require(uri.isAbsolute() && value.equals(uri.toASCIIString()), "Invalid external resource URI");
        if (!uri.getScheme().equalsIgnoreCase("pkg")) {
            return null;
        }
        PackageUrl purl = PackageUrl.parse(value);
        if (!purl.type().equals("janex")) {
            return null;
        }
        require(module && "java-module".equals(purl.namespace()) && purl.qualifiers().isEmpty()
                && purl.subpath() == null, "Janex Java module requirement is invalid in this path");
        return new ModuleRequirement(purl.name(), purl.version() == null ? "" : purl.version());
    }
}
