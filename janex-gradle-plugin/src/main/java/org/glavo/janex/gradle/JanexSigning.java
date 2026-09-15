// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.gradle;

import java.io.IOException;
import java.time.Instant;
import java.util.Arrays;

import org.gradle.api.file.RegularFileProperty;
import org.gradle.api.provider.Property;
import org.gradle.api.tasks.Input;
import org.gradle.api.tasks.InputFile;
import org.gradle.api.tasks.Internal;
import org.gradle.api.tasks.Optional;
import org.gradle.api.tasks.PathSensitive;
import org.gradle.api.tasks.PathSensitivity;
import org.glavo.janex.writer.CmsSigner;
import org.glavo.janex.writer.OpenPgpSigner;
import org.glavo.janex.writer.PackageSigner;
import org.glavo.janex.writer.SigningAlgorithm;

/// Configures optional publisher signing. Secret material is loaded only during task execution.
/// Signed tasks always run and do not use the build cache. Password values and decoded keys are
/// never stored in task properties or the configuration cache.
public abstract class JanexSigning {
    /// Creates empty signing options.
    public JanexSigning() { }

    /// Returns the optional PEM or DER CMS signer certificate; requires [#getCmsKey()].
    /// @return the certificate file property
    @InputFile
    @Optional
    @PathSensitive(PathSensitivity.NONE)
    public abstract RegularFileProperty getCmsCertificate();

    /// Returns the optional CMS private-key file; its contents are not Gradle task inputs.
    /// @return the private-key location property
    @Internal
    public abstract RegularFileProperty getCmsKey();

    /// Returns the optional transferable OpenPGP secret-key file, mutually exclusive with CMS.
    /// @return the secret-key location property
    @Internal
    public abstract RegularFileProperty getOpenPgpKey();

    /// Returns the optional complete hexadecimal OpenPGP primary or signing-subkey fingerprint.
    /// @return the key-selection property
    @Input
    @Optional
    public abstract Property<String> getOpenPgpFingerprint();

    /// Returns an explicit signature combination; absence selects the key's default.
    /// @return the algorithm-selection property
    @Input
    @Optional
    public abstract Property<SigningAlgorithm> getAlgorithm();

    /// Returns the name of the environment variable containing the key password, read during execution.
    /// Absence supplies no password; a configured variable must exist, and its exact value is used.
    /// @return the password environment-variable name property
    @Input
    @Optional
    public abstract Property<String> getPasswordEnvironment();

    /// Returns an optional ISO-8601 signing instant; absence uses the task execution time.
    /// Fixed times do not remove randomness from probabilistic signature algorithms.
    /// @return the signing-time property
    @Input
    @Optional
    public abstract Property<String> getTime();

    /// Returns whether a certificate or private-key location is configured.
    /// @return true when signing inputs are present
    @Internal
    public boolean isEnabled() {
        return getCmsCertificate().isPresent() || getCmsKey().isPresent() || getOpenPgpKey().isPresent();
    }

    /// Connects task conventions to the extension without resolving files or secrets.
    void convention(JanexSigning extension) {
        getCmsCertificate().convention(extension.getCmsCertificate());
        getCmsKey().convention(extension.getCmsKey());
        getOpenPgpKey().convention(extension.getOpenPgpKey());
        getOpenPgpFingerprint().convention(extension.getOpenPgpFingerprint());
        getAlgorithm().convention(extension.getAlgorithm());
        getPasswordEnvironment().convention(extension.getPasswordEnvironment());
        getTime().convention(extension.getTime());
    }

    /// Loads one signer at execution time, validating mutually exclusive and dependent options.
    PackageSigner load(Instant time) throws IOException {
        if (!isEnabled()) {
            if (getAlgorithm().isPresent() || getPasswordEnvironment().isPresent()
                    || getTime().isPresent() || getOpenPgpFingerprint().isPresent()) {
                throw new IOException("Signing options require a signing key");
            }
            return null;
        }
        boolean cms = getCmsCertificate().isPresent() || getCmsKey().isPresent();
        if (cms && (getOpenPgpKey().isPresent() || getOpenPgpFingerprint().isPresent()
                || !getCmsCertificate().isPresent() || !getCmsKey().isPresent())) {
            throw new IOException("Choose a CMS certificate and key, or an OpenPGP secret key");
        }
        char[] password = null;
        try {
            if (getPasswordEnvironment().isPresent()) {
                String value = System.getenv(getPasswordEnvironment().get());
                if (value == null) throw new IOException("Signing password environment variable is unset");
                password = value.toCharArray();
            }
            return cms ? CmsSigner.load(getCmsCertificate().get().getAsFile().toPath(),
                    getCmsKey().get().getAsFile().toPath(), password, getAlgorithm().getOrNull())
                    : OpenPgpSigner.load(getOpenPgpKey().get().getAsFile().toPath(), password,
                    getOpenPgpFingerprint().getOrNull(), getAlgorithm().getOrNull(), time);
        } finally {
            if (password != null) Arrays.fill(password, '\0');
        }
    }
}
