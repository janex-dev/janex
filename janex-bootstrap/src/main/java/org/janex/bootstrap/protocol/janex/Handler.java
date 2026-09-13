// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap.protocol.janex;

import java.io.IOException;
import java.net.URL;
import java.net.URLConnection;
import java.net.URLStreamHandler;

import org.janex.bootstrap.loader.ResourceLoader;

/// Resolves serialized Janex resource URLs through the active system loader.
public final class Handler extends URLStreamHandler {
    /// Creates the handler requested by the standard Java protocol lookup mechanism.
    public Handler() {
    }

    /// Opens the exact indexed resource without native path or network resolution.
    @Override
    protected URLConnection openConnection(URL url) throws IOException {
        return ResourceLoader.connect(url);
    }
}
