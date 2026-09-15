// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Clock;
import java.time.Instant;
import java.time.ZoneOffset;
import java.util.Arrays;
import java.util.Comparator;

import org.glavo.janex.reader.ContainerReader;
import org.glavo.janex.reader.JanexReader;

/// Exercises public fixture keys, encrypted loading, signature framing, and failure boundaries.
public final class SignatureTest {
    /// Prevents instantiation.
    private SignatureTest() { }

    /// Checks every supported signing combination without external cryptographic tools.
    /// @param args unused arguments
    /// @throws Exception if loading, signing, or an assertion fails
    public static void main(String[] args) throws Exception {
        Path fixtures = Path.of(System.getProperty("janex.test.fixtures"));
        Path root = Files.createTempDirectory("janex-signing-");
        Instant time = Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS);
        try {
            Path source = Files.createDirectory(root.resolve("input"));
            Files.writeString(source.resolve("data.txt"), "Signed resource");
            for (SigningAlgorithm algorithm : SigningAlgorithm.values()) {
                String name = fixture(algorithm);
                if (!name.equals("ed25519")) {
                    CmsSigner cms = CmsSigner.load(fixtures.resolve("cms/" + name + ".cert.pem"),
                            fixtures.resolve("cms/" + name + ".key.pem"), null, algorithm);
                    pack(source, root.resolve("cms-" + algorithm), cms, time, algorithm);
                    CmsSigner encrypted = CmsSigner.load(fixtures.resolve("cms/" + name + ".cert.pem"),
                            fixtures.resolve("cms/" + name + ".encrypted.pem"), "public-fixture-password".toCharArray(), algorithm);
                    encrypted.sign(new byte[]{0, 1, 2}, time);
                }
                OpenPgpSigner pgp = OpenPgpSigner.load(fixtures.resolve("openpgp/" + name + ".secret.pgp"),
                        null, null, algorithm, time);
                pack(source, root.resolve("pgp-" + algorithm), pgp, time, algorithm);
                OpenPgpSigner.load(fixtures.resolve("openpgp/" + name + ".secret.pgp"), null,
                        pgp.fingerprint().toUpperCase(java.util.Locale.ROOT), algorithm, time).sign(new byte[]{1}, time);
            }
            OpenPgpSigner.load(fixtures.resolve("openpgp/encrypted.secret.asc"),
                    "public-fixture-password".toCharArray(), null, null, time).sign(new byte[]{1}, time);
            fails(() -> OpenPgpSigner.load(fixtures.resolve("openpgp/encrypted.secret.pgp"),
                    "wrong".toCharArray(), null, null, time));
            fails(() -> OpenPgpSigner.load(fixtures.resolve("openpgp/expired.secret.pgp"), null, null, null, time));
            fails(() -> OpenPgpSigner.load(fixtures.resolve("openpgp/subkey.secret.pgp"), null, null, null, time));
            fails(() -> OpenPgpSigner.load(fixtures.resolve("openpgp/v6.secret.pgp"), null, null, null, time));
            fails(() -> OpenPgpSigner.load(fixtures.resolve("openpgp/ed25519.secret.pgp"), null, "00", null, time));
            CmsSigner cms = CmsSigner.load(fixtures.resolve("cms/rsa256.cert.pem"), fixtures.resolve("cms/rsa256.key.pem"), null, null);
            fails(() -> cms.sign(new byte[]{1}, Instant.parse("2200-01-01T00:00:00Z")));
            fails(() -> CmsSigner.load(fixtures.resolve("cms/rsa256.cert.pem"), fixtures.resolve("cms/rsa256.encrypted.pem"),
                    "wrong".toCharArray(), null));
            fails(() -> CmsSigner.load(fixtures.resolve("cms/rsa256.cert.pem"), fixtures.resolve("cms/rsa512.key.pem"),
                    null, null).sign(new byte[]{1}, time));
            fails(() -> CmsSigner.load(fixtures.resolve("cms/wrong-usage.cert.pem"), fixtures.resolve("cms/rsa256.key.pem"),
                    null, null).sign(new byte[]{1}, time));
            PackOptions incompatible = new PackOptions(source, root.resolve("incompatible"));
            incompatible.mainClass = "example.Main";
            incompatible.signer = cms;
            incompatible.withLauncher = true;
            fails(() -> JanexWriter.write(incompatible));
            require(!Files.exists(incompatible.output), "Unsupported wrapper published output");
            PackOptions failed = new PackOptions(source, root.resolve("failure"));
            failed.mainClass = "example.Main";
            int[] calls = {0};
            failed.signer = new PackageSigner() {
                /// Declares CMS signing.
                public int verificationType() { return 3; }
                /// Returns unused native-wrapper evidence.
                public byte[] certificate() { return new byte[]{1}; }
                /// Simulates a signing device failure after resource preparation.
                public byte[] sign(byte[] input, Instant instant) throws IOException {
                    calls[0]++;
                    throw new IOException("Signing device unavailable");
                }
            };
            fails(() -> JanexWriter.write(failed));
            require(calls[0] == 1 && !Files.exists(failed.output), "Signing failure published output or retried");
            System.out.println("Java signing checks passed.");
        } finally {
            try (var paths = Files.walk(root)) {
                for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) Files.deleteIfExists(path);
            }
        }
    }

    /// Returns the independent fixture family for a supported algorithm.
    public static String fixture(SigningAlgorithm algorithm) {
        return switch (algorithm) {
            case RSA_SHA256 -> "rsa256";
            case RSA_SHA512 -> "rsa512";
            case ECDSA_P256_SHA256 -> "p256";
            case ECDSA_P384_SHA384 -> "p384";
            case ED25519_SHA256, ED25519_SHA512 -> "ed25519";
        };
    }

    /// Checks signed container coverage and refusal to prepare without an authentication policy.
    private static void pack(Path source, Path output, PackageSigner signer, Instant time, SigningAlgorithm algorithm) throws Exception {
        PackOptions options = new PackOptions(source, output);
        options.mainClass = "example.Main";
        options.signer = signer;
        options.signingClock = Clock.fixed(time, ZoneOffset.UTC);
        JanexWriter.write(options);
        try (ContainerReader reader = new ContainerReader(output)) {
            require(reader.verifyChecksums().completeSecureCoverage(), "Missing signed content coverage");
            byte[] input = reader.verificationInput();
            require(input[input.length - 1] == signer.verificationType(), "Wrong protected verification tag");
            require(reader.verification().payload().length != 0, "Missing signature");
            byte[] again = signer.sign(input, time);
            if (algorithm.name().startsWith("RSA_") || algorithm.name().startsWith("ED25519_")) {
                require(Arrays.equals(again, reader.verification().payload()), "Deterministic signing changed with a fixed time");
            }
        }
        fails(() -> { try (JanexReader ignored = new JanexReader(output)) { } });
    }

    /// Represents an operation that must fail without publishing output.
    private interface Action {
        /// Performs the rejected operation.
        void run() throws IOException;
    }

    /// Requires a checked failure for invalid keys, policies, or writer options.
    private static void fails(Action action) throws IOException {
        try { action.run(); } catch (IOException expected) { return; }
        throw new AssertionError("Invalid signing operation succeeded");
    }

    /// Reports a failed behavioral assertion independently of JVM assertion flags.
    private static void require(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
}
