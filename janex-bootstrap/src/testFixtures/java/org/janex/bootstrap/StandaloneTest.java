// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.nio.file.Paths;
import java.util.Arrays;

/// Invokes standalone launching without requiring the initial JVM to parse a prefixed ZIP64 JAR.
public final class StandaloneTest {
    /// Prevents instantiation.
    private StandaloneTest() {
    }

    /// Launches the first argument as a Janex file and forwards all remaining arguments.
    ///
    /// @param arguments Janex file path followed by program arguments
    /// @throws Exception if launch preparation or process execution fails
    public static void main(String[] arguments) throws Exception {
        System.exit(Standalone.launch(Paths.get(arguments[0]), Arrays.copyOfRange(arguments, 1, arguments.length)));
    }
}
