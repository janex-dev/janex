// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.nio.file.Path;
import java.time.Instant;
import java.util.Arrays;
import java.util.Comparator;
import java.util.Date;
import java.util.HexFormat;

import org.bouncycastle.bcpg.S2K;
import org.bouncycastle.openpgp.PGPObjectFactory;
import org.bouncycastle.openpgp.PGPPrivateKey;
import org.bouncycastle.openpgp.PGPPublicKey;
import org.bouncycastle.openpgp.PGPSecretKeyRing;
import org.bouncycastle.openpgp.PGPSignature;
import org.bouncycastle.openpgp.PGPSignatureGenerator;
import org.bouncycastle.openpgp.PGPSignatureSubpacketGenerator;
import org.bouncycastle.openpgp.PGPUtil;
import org.bouncycastle.openpgp.api.OpenPGPCertificate;
import org.bouncycastle.openpgp.api.bc.BcOpenPGPImplementation;
import org.bouncycastle.openpgp.operator.bc.BcKeyFingerprintCalculator;
import org.bouncycastle.openpgp.operator.bc.BcPBESecretKeyDecryptorBuilder;
import org.bouncycastle.openpgp.operator.bc.BcPGPContentSignerBuilder;
import org.bouncycastle.openpgp.operator.bc.BcPGPContentVerifierBuilderProvider;
import org.bouncycastle.openpgp.operator.bc.BcPGPDigestCalculatorProvider;
import org.bouncycastle.openpgp.operator.jcajce.JcaPGPKeyConverter;

import static org.glavo.janex.reader.internal.Input.require;

/// Signs exact binary documents with a certified OpenPGP primary key or signing subkey.
/// Certifications, bindings, signing flags, expiration, and supplied revocations are evaluated
/// by Bouncy Castle's certificate policy. Callers authorize the certificate and supply current
/// revocation data; no network requests are made. The decoded private key is retained.
public final class OpenPgpSigner implements PackageSigner {
    /// Transferable public evidence retained for authorization checks and native wrapping.
    private final OpenPGPCertificate certificate;
    /// Selected public signing key.
    private final PGPPublicKey publicKey;
    /// Unlocked private signing key.
    private final PGPPrivateKey privateKey;
    /// Checked signature and digest combination.
    private final SigningAlgorithm algorithm;

    /// Retains one unlocked key and its public certificate.
    private OpenPgpSigner(OpenPGPCertificate certificate, PGPPublicKey publicKey,
                          PGPPrivateKey privateKey, SigningAlgorithm algorithm) {
        this.certificate = certificate;
        this.publicKey = publicKey;
        this.privateKey = privateKey;
        this.algorithm = algorithm;
    }

    /// Loads one binary or armored transferable secret certificate, bounded to 8 MiB.
    /// Automatic selection prefers the newest eligible secret signing subkey, then the primary
    /// key; equal creation times use ascending fingerprints. Passwords are not retained.
    /// Iterated S2K is limited to 64 MiB of hashing; Argon2 to 64 MiB, ten passes, and 16 lanes.
    /// @param file secret-certificate path
    /// @param password decryption password, or null for an unencrypted key; not modified
    /// @param fingerprint complete hexadecimal signing-key fingerprint, or null for automatic selection
    /// @param algorithm explicit combination, or null for the key's default
    /// @param time instant used to evaluate the certificate before unlocking
    /// @return a signer that rechecks authorization at each signing instant
    /// @throws IOException if parsing, authorization, selection, or decryption fails
    public static OpenPgpSigner load(Path file, char[] password, String fingerprint,
                                     SigningAlgorithm algorithm, Instant time) throws IOException {
        byte[] encoded = Resources.read(file, SigningSupport.KEY_LIMIT);
        char[] secret = password == null ? new char[0] : password.clone();
        try (var input = PGPUtil.getDecoderStream(new ByteArrayInputStream(encoded))) {
            PGPObjectFactory objects = new PGPObjectFactory(input, new BcKeyFingerprintCalculator());
            Object object = objects.nextObject();
            require(object instanceof PGPSecretKeyRing && objects.nextObject() == null,
                    "Expected one transferable OpenPGP secret certificate");
            PGPSecretKeyRing ring = (PGPSecretKeyRing) object;
            OpenPGPCertificate certificate = new OpenPGPCertificate(ring, new BcOpenPGPImplementation());
            byte[] selected = fingerprint == null ? null : HexFormat.of().parseHex(fingerprint);
            var keys = certificate.getSigningKeys(Date.from(time)).stream()
                    .map(OpenPGPCertificate.OpenPGPComponentKey::getPGPPublicKey)
                    .sorted(Comparator.comparing(PGPPublicKey::isMasterKey)
                            .thenComparing(PGPPublicKey::getCreationTime, Comparator.reverseOrder())
                            .thenComparing(key -> HexFormat.of().formatHex(key.getFingerprint())))
                    .toList();
            for (PGPPublicKey key : keys) {
                if (selected != null && !Arrays.equals(selected, key.getFingerprint())) continue;
                var privatePart = ring.getSecretKey(key.getKeyIdentifier());
                if (privatePart == null || privatePart.isPrivateKeyEmpty()) continue;
                require(key.getVersion() == 4 || key.getVersion() == 6, "Unsupported OpenPGP key version");
                SigningAlgorithm choice;
                try {
                    choice = SigningSupport.algorithm(new JcaPGPKeyConverter().setProvider(SigningSupport.PROVIDER)
                            .getPublicKey(key), algorithm, true);
                } catch (IOException unsupported) {
                    if (selected != null) throw unsupported;
                    continue;
                }
                if (key.getVersion() == 6 && choice == SigningAlgorithm.ED25519_SHA256) {
                    require(algorithm == null, "Version-6 Ed25519 signatures require SHA-512");
                    choice = SigningAlgorithm.ED25519_SHA512;
                }
                S2K s2k = privatePart.getS2K();
                if (s2k != null) {
                    require(s2k.getType() != S2K.SALTED_AND_ITERATED || s2k.getIterationCount() <= 64L * 1024 * 1024,
                            "OpenPGP S2K work limit exceeded");
                    require(s2k.getType() != S2K.ARGON_2 || s2k.getMemorySizeExponent() <= 16
                            && s2k.getPasses() <= 10 && s2k.getParallelism() <= 16, "OpenPGP Argon2 work limit exceeded");
                }
                PGPPrivateKey unlocked = privatePart.extractPrivateKey(new BcPBESecretKeyDecryptorBuilder(
                        new BcPGPDigestCalculatorProvider()).build(secret));
                require(unlocked != null, "Missing OpenPGP private key");
                return new OpenPgpSigner(new OpenPGPCertificate(certificate.getPGPPublicKeyRing(),
                        new BcOpenPGPImplementation()), key, unlocked, choice);
            }
            throw new IOException("No authorized OpenPGP signing key matches the selection");
        } catch (Exception failure) {
            throw new IOException("Cannot load OpenPGP signing key", failure);
        } finally {
            Arrays.fill(encoded, (byte) 0);
            Arrays.fill(secret, '\0');
        }
    }

    /// Returns the OpenPGP verification tag, 2.
    @Override
    public int verificationType() { return 2; }

    /// Returns a new binary transferable public certificate for the native launcher's trust pin.
    @Override
    public byte[] certificate() throws IOException { return certificate.getEncoded(); }

    /// Returns the complete lowercase hexadecimal fingerprint of the selected signing key.
    /// @return the full signing-key fingerprint
    public String fingerprint() { return HexFormat.of().formatHex(publicKey.getFingerprint()); }

    /// Produces one binary Signature packet with protected creation time and issuer fingerprint.
    @Override
    public synchronized byte[] sign(byte[] input, Instant time) throws IOException {
        try {
            require(time.getEpochSecond() >= 0 && time.getEpochSecond() <= 0xffff_ffffL,
                    "OpenPGP signing time is outside the unsigned 32-bit range");
            require(certificate.getSigningKeys(Date.from(time)).stream().anyMatch(key ->
                    Arrays.equals(key.getPGPPublicKey().getFingerprint(), publicKey.getFingerprint())),
                    "OpenPGP signing key is no longer authorized");
            PGPSignatureGenerator generator = new PGPSignatureGenerator(new BcPGPContentSignerBuilder(
                    publicKey.getAlgorithm(), algorithm.hash), publicKey);
            generator.init(PGPSignature.BINARY_DOCUMENT, privateKey);
            PGPSignatureSubpacketGenerator hashed = new PGPSignatureSubpacketGenerator();
            hashed.setSignatureCreationTime(false, Date.from(time));
            hashed.setIssuerFingerprint(false, publicKey);
            generator.setHashedSubpackets(hashed.generate());
            generator.update(input);
            PGPSignature signature = generator.generate();
            signature.init(new BcPGPContentVerifierBuilderProvider(), publicKey);
            signature.update(input);
            require(signature.verify(), "OpenPGP private key does not match its certificate");
            return signature.getEncoded();
        } catch (Exception failure) {
            throw new IOException("OpenPGP signing failed", failure);
        }
    }
}
