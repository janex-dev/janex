// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.example;

/// Prints the preset and user-supplied arguments of the packaged application.
public final class Hello {
    /// Prevents instantiation of the application entry point.
    private Hello() {
    }

    /// Prints each argument on its own line, in the order supplied by the launcher.
    ///
    /// @param args preset arguments followed by user-supplied arguments
    public static void main(String[] args) {
        for (String argument : args) {
            System.out.println(argument);
        }
    }
}
