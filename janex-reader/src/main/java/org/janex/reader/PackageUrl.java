// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.*;

import org.janex.reader.internal.Input;
import org.janex.reader.internal.UnicodeCaseChecks;

import static org.janex.reader.internal.Input.require;

/// Retains a canonical ECMA-427 Package URL without acquiring the named package.
///
/// Parsing checks component encodings and the registered type constraints supported by the
/// native reader. Unknown types retain the generic component rules. Instances are immutable.
public final class PackageUrl {
    /// Original canonical ASCII spelling.
    private final String encoded;
    /// Types that prohibit namespaces.
    private static final Set<String> NO_NAMESPACE = set("bitnami", "cargo", "chrome-extension", "cocoapods",
            "conda", "cran", "gem", "hackage", "julia", "mlflow", "nuget", "oci", "otp", "pub", "pypi", "vcpkg");
    /// Types that require namespaces.
    private static final Set<String> REQUIRE_NAMESPACE = set("composer", "cpan", "huggingface", "swift", "vscode-extension");
    /// Types with lowercase canonical names.
    private static final Set<String> LOWER_NAME = set("bitbucket", "composer", "deb", "github", "hex", "npm", "pypi");
    /// Types with lowercase canonical namespaces.
    private static final Set<String> LOWER_NAMESPACE = set("apk", "bitbucket", "composer", "deb", "github", "golang", "hex", "qpkg", "rpm");
    /// Lowercase ASCII package type.
    private final String type;
    /// Decoded namespace, or null when omitted.
    private final String namespace;
    /// Decoded nonempty package name.
    private final String name;
    /// Decoded nonempty version, or null when omitted.
    private final String version;
    /// Decoded nonempty subpath, or null when omitted.
    private final String subpath;
    /// Immutable qualifiers in canonical key order.
    private final Map<String, String> qualifiers;

    /// Retains already validated component values.
    private PackageUrl(String encoded, String type, String namespace, String name, String version, String subpath,
                       Map<String, String> qualifiers) {
        this.encoded = encoded;
        this.type = type;
        this.namespace = namespace;
        this.name = name;
        this.version = version;
        this.subpath = subpath;
        this.qualifiers = Collections.unmodifiableMap(qualifiers);
    }

    /// Parses one canonical Package URL; input requiring normalization is rejected.
    ///
    /// Percent escapes use uppercase hexadecimal UTF-8 bytes. Unreserved characters and colons
    /// remain literal. Qualifier keys are unique and sorted. Empty optional components must omit
    /// their separators. Decoded values retain their exact Unicode spelling.
    ///
    /// @param value canonical ASCII Package URL, nonnull
    /// @return immutable decoded components
    /// @throws IOException if syntax, encoding, canonical spelling, or a type constraint is invalid
    public static PackageUrl parse(String value) throws IOException {
        require(value.startsWith("pkg:"), "Invalid Package URL scheme");
        String rest = value.substring(4);
        String subpath = null;
        int separator = rest.indexOf('#');
        if (separator >= 0) {
            subpath = segments(rest.substring(separator + 1), true);
            rest = rest.substring(0, separator);
        }
        Map<String, String> qualifiers = new LinkedHashMap<String, String>();
        separator = rest.indexOf('?');
        if (separator >= 0) {
            String previous = null;
            for (String pair : rest.substring(separator + 1).split("&", -1)) {
                int equals = pair.indexOf('=');
                require(equals > 0, "Invalid Package URL qualifier");
                String key = pair.substring(0, equals);
                token(key, true);
                require(previous == null || previous.compareTo(key) < 0, "Package URL qualifiers must be unique and sorted");
                qualifiers.put(key, component(pair.substring(equals + 1)));
                previous = key;
            }
            rest = rest.substring(0, separator);
        }
        String version = null;
        separator = rest.indexOf('@');
        if (separator >= 0) {
            version = component(rest.substring(separator + 1));
            rest = rest.substring(0, separator);
        }
        separator = rest.indexOf('/');
        require(separator > 0, "Missing Package URL type");
        String type = rest.substring(0, separator);
        token(type, false);
        String path = rest.substring(separator + 1);
        separator = path.lastIndexOf('/');
        String namespace = separator < 0 ? null : segments(path.substring(0, separator), false);
        String name = component(path.substring(separator + 1));
        require(!NO_NAMESPACE.contains(type) || namespace == null, "Package URL type prohibits a namespace");
        require(!REQUIRE_NAMESPACE.contains(type) || namespace != null, "Package URL type requires a namespace");
        boolean lowerName = LOWER_NAME.contains(type) || type.equals("mlflow")
                && qualifiers.getOrDefault("repository_url", "").contains("databricks");
        require(!lowerName || lowercase(name), "Noncanonical Package URL name case");
        require(!LOWER_NAMESPACE.contains(type) || namespace == null || lowercase(namespace), "Noncanonical Package URL namespace case");
        require(!type.equals("huggingface") || version == null || lowercase(version), "Noncanonical Package URL version case");
        require(!type.equals("pypi") || name.indexOf('_') < 0, "Noncanonical PyPI name");
        require(!type.equals("cpan") || !name.contains("::"), "Invalid CPAN distribution name");
        require(!type.equals("julia") || qualifiers.containsKey("uuid"), "Julia Package URL requires a uuid qualifier");
        if (type.equals("chrome-extension")) {
            require(name.matches("[a-z]{32}"), "Invalid Chrome extension identifier");
            require(version == null || version.matches("[0-9]+(?:\\.[0-9]+){0,3}"), "Invalid Chrome extension version");
        }
        return new PackageUrl(value, type, namespace, name, version, subpath, qualifiers);
    }

    /// Returns the original canonical ASCII spelling.
    @Override
    public String toString() {
        return encoded;
    }

    /// Returns the lowercase ASCII package type.
    public String type() {
        return type;
    }

    /// Returns the decoded namespace, or null when omitted.
    public String namespace() {
        return namespace;
    }

    /// Returns the decoded nonempty package name.
    public String name() {
        return name;
    }

    /// Returns the decoded version, or null when omitted.
    public String version() {
        return version;
    }

    /// Returns the decoded relative subpath, or null when omitted.
    public String subpath() {
        return subpath;
    }

    /// Returns immutable decoded qualifiers in canonical key order.
    public Map<String, String> qualifiers() {
        return qualifiers;
    }

    /// Creates a private constant set of registered types.
    private static Set<String> set(String... values) {
        return new HashSet<String>(Arrays.asList(values));
    }

    /// Checks culture-independent lowercase spelling.
    private static boolean lowercase(String value) {
        return UnicodeCaseChecks.isLowercase(value);
    }

    /// Validates lowercase ASCII type names and qualifier keys.
    private static void token(String value, boolean qualifier) throws IOException {
        require(!value.isEmpty() && value.charAt(0) >= 'a' && value.charAt(0) <= 'z', "Invalid Package URL token");
        for (int i = 1; i < value.length(); i++) {
            char ch = value.charAt(i);
            require(ch >= 'a' && ch <= 'z' || ch >= '0' && ch <= '9' || ch == '.' || ch == '-'
                    || qualifier && ch == '_', "Invalid Package URL token");
        }
    }

    /// Tests characters permitted literally inside canonical components.
    private static boolean literal(int ch) {
        return ch >= 'a' && ch <= 'z' || ch >= 'A' && ch <= 'Z' || ch >= '0' && ch <= '9'
                || ch == '.' || ch == '-' || ch == '_' || ch == '~' || ch == ':';
    }

    /// Decodes a nonempty canonical UTF-8 component without converting plus signs to spaces.
    private static String component(String value) throws IOException {
        require(!value.isEmpty(), "Empty Package URL component");
        ByteArrayOutputStream bytes = new ByteArrayOutputStream(value.length());
        for (int i = 0; i < value.length(); i++) {
            int ch = value.charAt(i);
            if (ch == '%') {
                require(i + 2 < value.length(), "Truncated Package URL escape");
                ch = hex(value.charAt(++i)) * 16 + hex(value.charAt(++i));
                require(!literal(ch), "Unnecessary Package URL escape");
            } else {
                require(literal(ch), "Unescaped Package URL character");
            }
            bytes.write(ch);
        }
        return Input.utf8(bytes.toByteArray());
    }

    /// Decodes one uppercase hexadecimal digit.
    private static int hex(char ch) throws IOException {
        if (ch >= '0' && ch <= '9') {
            return ch - '0';
        }
        require(ch >= 'A' && ch <= 'F', "Invalid Package URL escape");
        return ch - 'A' + 10;
    }

    /// Decodes slash-separated namespace or subpath components.
    private static String segments(String value, boolean subpath) throws IOException {
        StringBuilder result = new StringBuilder();
        for (String part : value.split("/", -1)) {
            String decoded = component(part);
            require(decoded.indexOf('/') < 0 && (!subpath || !decoded.equals(".") && !decoded.equals("..")),
                    "Invalid Package URL path segment");
            if (result.length() != 0) {
                result.append('/');
            }
            result.append(decoded);
        }
        return result.toString();
    }
}
