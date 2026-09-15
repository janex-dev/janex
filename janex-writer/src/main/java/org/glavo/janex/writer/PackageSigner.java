// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.time.Instant;

/// Signs the exact Janex metadata verification input with a caller-authorized publisher key.
/// Implementations must enforce their key, algorithm, time, and revocation policies and produce
/// the Janex profile for their declared mechanism. A failure must throw; unsigned fallback is forbidden.
public interface PackageSigner {
    /// Returns the stable verification tag: 2 for OpenPGP or 3 for CMS.
    /// @return the signature mechanism tag
    int verificationType();

    /// Returns owned public trust material for native wrapping: a DER CMS certificate or a
    /// transferable binary OpenPGP public certificate. It must authorize this signer's key.
    /// @return public certificate bytes, never private key material
    /// @throws IOException if the public certificate cannot be encoded
    byte[] certificate() throws IOException;

    /// Returns one nonempty detached signature payload for the supplied exact input.
    /// The writer invokes this once after choosing the resource representation.
    /// Input must remain unchanged during the call; implementations must not modify or retain it.
    /// @param input complete verification input, including the declared verification tag
    /// @param time signing instant selected by the caller
    /// @return an owned signature byte array
    /// @throws IOException if key policy or signature generation fails
    byte[] sign(byte[] input, Instant time) throws IOException;
}
