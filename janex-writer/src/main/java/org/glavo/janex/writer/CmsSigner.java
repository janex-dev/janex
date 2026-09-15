// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.nio.file.Path;
import java.security.PrivateKey;
import java.security.cert.X509Certificate;
import java.time.Instant;
import java.util.Arrays;
import java.util.Date;
import java.util.List;
import java.util.Objects;
import java.util.Set;

import org.bouncycastle.asn1.DERSet;
import org.bouncycastle.asn1.cms.Attribute;
import org.bouncycastle.asn1.cms.AttributeTable;
import org.bouncycastle.asn1.cms.CMSAttributes;
import org.bouncycastle.asn1.cms.Time;
import org.bouncycastle.asn1.pkcs.PrivateKeyInfo;
import org.bouncycastle.cert.X509CertificateHolder;
import org.bouncycastle.cert.jcajce.JcaX509CertificateConverter;
import org.bouncycastle.cms.CMSProcessableByteArray;
import org.bouncycastle.cms.CMSSignedDataGenerator;
import org.bouncycastle.cms.DefaultSignedAttributeTableGenerator;
import org.bouncycastle.cms.jcajce.JcaSignerInfoGeneratorBuilder;
import org.bouncycastle.cms.jcajce.JcaSimpleSignerInfoVerifierBuilder;
import org.bouncycastle.openssl.PEMEncryptedKeyPair;
import org.bouncycastle.openssl.PEMKeyPair;
import org.bouncycastle.openssl.jcajce.JcaPEMKeyConverter;
import org.bouncycastle.openssl.jcajce.JceOpenSSLPKCS8DecryptorProviderBuilder;
import org.bouncycastle.openssl.jcajce.JcePEMDecryptorProviderBuilder;
import org.bouncycastle.operator.jcajce.JcaContentSignerBuilder;
import org.bouncycastle.operator.jcajce.JcaDigestCalculatorProviderBuilder;
import org.bouncycastle.pkcs.PKCS8EncryptedPrivateKeyInfo;

import static org.glavo.janex.reader.internal.Input.require;

/// Produces detached DER CMS signatures with protected content and signature algorithms.
/// Checks certificate validity, signing usage, key strength, and private-key correspondence.
/// The caller authorizes the certificate and supplies current revocation policy; this signer
/// performs no issuer-chain building or network revocation lookup. Instances retain private keys.
public final class CmsSigner implements PackageSigner {
    /// Caller-authorized signing certificate.
    private final X509Certificate certificate;
    /// Matching private key.
    private final PrivateKey key;
    /// Checked signing algorithm.
    private final SigningAlgorithm algorithm;

    /// Creates a signer with a nonnull certificate and private key.
    /// @param certificate publisher certificate; must not be mutated during signing
    /// @param key private key retained by this signer
    /// @param algorithm explicit combination, or null to select the key's default
    /// @throws IOException if the certificate key or algorithm is unsupported
    public CmsSigner(X509Certificate certificate, PrivateKey key, SigningAlgorithm algorithm) throws IOException {
        this.certificate = Objects.requireNonNull(certificate);
        this.key = Objects.requireNonNull(key);
        this.algorithm = SigningSupport.algorithm(certificate.getPublicKey(), algorithm, false);
    }

    /// Loads one PEM or DER certificate and PKCS#8 private key, each bounded to 8 MiB.
    /// Encrypted PKCS#8 and traditional PEM key pairs are supported. Passwords are not retained.
    /// @param certificateFile certificate path
    /// @param keyFile private-key path
    /// @param password decryption password, or null for an unencrypted key; not modified
    /// @param algorithm explicit combination, or null for automatic selection
    /// @return a signer retaining the decoded private key
    /// @throws IOException if parsing, decryption, or algorithm selection fails
    public static CmsSigner load(Path certificateFile, Path keyFile, char[] password, SigningAlgorithm algorithm) throws IOException {
        byte[] encoded = Resources.read(keyFile, SigningSupport.KEY_LIMIT);
        char[] secret = password == null ? new char[0] : password.clone();
        try {
            byte[] certBytes = Resources.read(certificateFile, SigningSupport.KEY_LIMIT);
            Object certObject = SigningSupport.pem(certBytes);
            X509CertificateHolder holder = certObject == null ? new X509CertificateHolder(certBytes)
                    : (X509CertificateHolder) certObject;
            Object parsed = SigningSupport.pem(encoded);
            if (parsed == null) {
                try { parsed = PrivateKeyInfo.getInstance(encoded); }
                catch (IllegalArgumentException invalid) { parsed = new PKCS8EncryptedPrivateKeyInfo(encoded); }
            }
            if (parsed instanceof PKCS8EncryptedPrivateKeyInfo encrypted) {
                parsed = encrypted.decryptPrivateKeyInfo(new JceOpenSSLPKCS8DecryptorProviderBuilder()
                        .setProvider(SigningSupport.PROVIDER).build(secret));
            } else if (parsed instanceof PEMEncryptedKeyPair encrypted) {
                parsed = encrypted.decryptKeyPair(new JcePEMDecryptorProviderBuilder()
                        .setProvider(SigningSupport.PROVIDER).build(secret));
            }
            if (parsed instanceof PEMKeyPair pair) parsed = pair.getPrivateKeyInfo();
            PrivateKey key = new JcaPEMKeyConverter().setProvider(SigningSupport.PROVIDER)
                    .getPrivateKey((PrivateKeyInfo) parsed);
            return new CmsSigner(new JcaX509CertificateConverter().setProvider(SigningSupport.PROVIDER)
                    .getCertificate(holder), key, algorithm);
        } catch (Exception failure) {
            throw new IOException("Cannot load CMS signing key or certificate", failure);
        } finally {
            Arrays.fill(encoded, (byte) 0);
            Arrays.fill(secret, '\0');
        }
    }

    /// Returns the CMS verification tag, 3.
    @Override
    public int verificationType() { return 3; }

    /// Returns a new DER certificate array for the native launcher's explicit signer pin.
    @Override
    public byte[] certificate() throws IOException {
        try { return certificate.getEncoded(); }
        catch (Exception failure) { throw new IOException("Cannot encode CMS certificate", failure); }
    }

    /// Signs at the supplied instant after validating the certificate and matching private key.
    @Override
    public byte[] sign(byte[] input, Instant time) throws IOException {
        try {
            Date date = Date.from(time);
            certificate.checkValidity(date);
            Set<String> critical = certificate.getCriticalExtensionOIDs();
            require(critical == null || Set.of("2.5.29.15", "2.5.29.19", "2.5.29.37").containsAll(critical),
                    "Unsupported critical certificate extension");
            boolean[] usage = certificate.getKeyUsage();
            require(usage == null || usage[0], "Certificate does not permit digital signatures");
            List<String> extended = certificate.getExtendedKeyUsage();
            require(extended == null || extended.contains("1.3.6.1.5.5.7.3.3") || extended.contains("2.5.29.37.0"),
                    "Certificate does not permit code signing");
            var builder = new JcaSignerInfoGeneratorBuilder(new JcaDigestCalculatorProviderBuilder()
                    .setProvider(SigningSupport.PROVIDER).build());
            builder.setSignedAttributeGenerator(new DefaultSignedAttributeTableGenerator(new AttributeTable(
                    new Attribute(CMSAttributes.signingTime, new DERSet(new Time(date))))));
            CMSSignedDataGenerator generator = new CMSSignedDataGenerator();
            generator.addSignerInfoGenerator(builder.build(new JcaContentSignerBuilder(algorithm.jca)
                    .setProvider(SigningSupport.PROVIDER).build(key), certificate));
            generator.addCertificate(new X509CertificateHolder(certificate.getEncoded()));
            var signed = generator.generate(new CMSProcessableByteArray(input), false);
            var signer = signed.getSignerInfos().getSigners().iterator().next();
            require(signer.verify(new JcaSimpleSignerInfoVerifierBuilder().setProvider(SigningSupport.PROVIDER)
                    .build(certificate)), "Private key does not match the CMS certificate");
            return signed.toASN1Structure().getEncoded("DER");
        } catch (Exception failure) {
            throw new IOException("CMS signing failed", failure);
        }
    }
}
