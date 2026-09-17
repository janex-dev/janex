// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal;

import java.io.IOException;
import java.util.*;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.glavo.janex.reader.internal.Input.*;

/// Evaluates conditions against the current Java process and the run invocation channel.
public final class Conditions {
    /// Prevents instantiation.
    private Conditions() {
    }

    /// Validates a condition and returns whether it matches this runtime.
    public static boolean matches(Map<Object, Object> condition) throws IOException {
        return matches(condition, null);
    }

    /// Evaluates a condition using OS, architecture, invocation, Java version, and vendor overrides.
    /// A null context uses the current JVM and the run invocation channel.
    public static boolean matches(Map<Object, Object> condition, String[] context) throws IOException {
        return evaluate(condition, true, context);
    }

    /// Validates all known condition fields without consulting the current process.
    public static void validate(Map<Object, Object> condition) throws IOException {
        evaluate(condition, false, null);
    }

    /// Validates a condition and optionally compares it with current process properties.
    private static boolean evaluate(Map<Object, Object> condition, boolean current, String[] context) throws IOException {
        integers(condition);
        require(!has(condition, 0) && !has(condition, 3), "Reserved condition key");
        String os = current ? System.getProperty("os.name") : "";
        os = os.startsWith("Windows") ? "windows" : os.equals("Mac OS X") ? "macos" : os.equals("Linux") ? "linux"
                : os.equals("FreeBSD") ? "freebsd" : os;
        String arch = current ? System.getProperty("os.arch") : "";
        arch = arch.equals("amd64") || arch.equals("x86_64") ? "x86-64"
                : arch.matches("i[3-6]86") ? "x86" : arch.equals("arm64") ? "aarch64" : arch;
        String version = current ? System.getProperty("java.version") : "8";
        String vendor = current ? System.getProperty("java.vendor") : "";
        if (context != null) {
            os = context[0];
            arch = context[1];
            version = context[3];
            vendor = context[4];
        }
        boolean result = true;
        if (has(condition, 1)) {
            result &= selector(get(condition, 1), os);
        }
        if (has(condition, 2)) {
            result &= selector(get(condition, 2), arch);
        }
        if (has(condition, 4)) {
            result &= selector(get(condition, 4), context == null ? "run" : context[2]);
        }
        if (has(condition, 5)) {
            Map<Object, Object> runtime = integers(map(get(condition, 5)));
            String type = nonempty(get(runtime, 0));
            Map<Object, Object> requirements = integers(map(get(runtime, 1)));
            if (type.equals("janex.java")) {
                if (has(requirements, 0)) {
                    result &= range(nonempty(get(requirements, 0)), version);
                }
                if (has(requirements, 1)) {
                    result &= nonempty(get(requirements, 1)).equals(vendor);
                }
            } else {
                result = false;
            }
        }
        return result;
    }

    /// Requires a nonempty CBOR text string.
    public static String nonempty(Object value) throws IOException {
        String result = text(value);
        require(!result.isEmpty(), "Expected nonempty text");
        return result;
    }

    /// Validates and matches a name selector without skipping unmatched array members.
    private static boolean selector(Object value, String expected) throws IOException {
        if (value instanceof String) {
            return nonempty(value).equals(expected);
        }
        List<Object> values = list(value);
        require(!values.isEmpty(), "Empty name selector");
        boolean result = false;
        for (Object item : values) {
            result |= nonempty(item).equals(expected);
        }
        return result;
    }

    /// Returns the current runtime's feature version.
    public static int feature() throws IOException {
        return feature(System.getProperty("java.version"));
    }

    /// Returns the feature version of a caller-selected Java runtime version.
    public static int feature(String version) throws IOException {
        return new Version(version, false).numbers[0];
    }

    /// Validates a canonical VERS timeline and tests one Java version.
    public static boolean range(String text, String candidate) throws IOException {
        Version current = new Version(candidate, false);
        require(text.startsWith("vers:jep322/") && text.matches("[!-~]+"), "Invalid Java VERS");
        String body = text.substring(12);
        if (body.equals("*")) {
            return true;
        }
        List<Version> versions = new ArrayList<Version>();
        List<String> operators = new ArrayList<String>();
        String previous = null;
        String boundary = null;
        for (String token : body.split("\\|", -1)) {
            Matcher matcher = Pattern.compile("(>=|<=|!=|>|<|=)?(.+)").matcher(token);
            require(matcher.matches(), "Invalid VERS constraint");
            String operator = matcher.group(1) == null ? "=" : matcher.group(1);
            Version version = new Version(matcher.group(2), true);
            require(versions.isEmpty() || versions.get(versions.size() - 1).compareTo(version) < 0,
                    "VERS constraints must be strictly increasing");
            if (!operator.equals("!=")) {
                require(!"=".equals(previous) || !operator.startsWith("<"), "VERS equality before upper bound");
                previous = operator;
            }
            if (operator.startsWith(">") || operator.startsWith("<")) {
                require(boundary == null || boundary.startsWith(">") != operator.startsWith(">"), "VERS bounds must alternate");
                boundary = operator;
            }
            versions.add(version);
            operators.add(operator);
        }
        boolean included = true;
        for (String operator : operators) {
            if (!operator.equals("!=")) {
                included = operator.startsWith("<");
                break;
            }
        }
        for (int i = 0; i < versions.size(); i++) {
            int order = current.compareTo(versions.get(i));
            String operator = operators.get(i);
            if (order < 0) {
                break;
            }
            if (order == 0) {
                return operator.equals("=") || operator.equals(">=") || operator.equals("<=");
            }
            if (operator.startsWith(">")) {
                included = true;
            }
            if (operator.startsWith("<")) {
                included = false;
            }
        }
        return included;
    }

    /// Java version ordering without build or optional information.
    private static final class Version implements Comparable<Version> {
        /// Numeric components with implicit trailing zeros.
        final int[] numbers;
        /// Prerelease text, or null for a release.
        final String pre;

        /// Parses a Java version, expanding Java 8 aliases.
        Version(String value, boolean canonical) throws IOException {
            if (value.startsWith("8u")) {
                decimal(value.substring(2));
                value = "8.0." + value.substring(2);
                canonical = false;
            } else if (value.startsWith("1.8.0")) {
                String alias = value.substring(5);
                String update = null;
                String suffix = null;
                if (alias.startsWith("_")) {
                    int separator = alias.indexOf('-');
                    update = alias.substring(1, separator < 0 ? alias.length() : separator);
                    decimal(update);
                    if (separator >= 0) {
                        suffix = alias.substring(separator + 1);
                        if (suffix.startsWith("b") && numeric(suffix.substring(1))) {
                            decimal(suffix.substring(1));
                            suffix = null;
                        } else {
                            versionText(suffix, false);
                        }
                    }
                } else if (alias.startsWith("-")) {
                    suffix = alias.substring(1);
                    versionText(suffix, false);
                } else {
                    require(alias.isEmpty(), "Invalid Java 8 alias");
                }
                value = update == null ? "8" : "8.0." + update;
                if (suffix != null) {
                    value += "-" + suffix;
                }
                canonical = false;
            }
            int plus = value.indexOf('+');
            String beforeBuild = plus < 0 ? value : value.substring(0, plus);
            String build = plus < 0 ? null : value.substring(plus + 1);
            String[] prefix = beforeBuild.split("-", 3);
            String prerelease = prefix.length > 1 ? prefix[1] : null;
            String optional = prefix.length > 2 ? prefix[2] : null;
            String[] components = prefix[0].split("\\.", -1);
            numbers = new int[components.length];
            for (int i = 0; i < numbers.length; i++) {
                numbers[i] = decimal(components[i]);
            }
            require(numbers[0] >= 8, "Java 8 or later is required");
            require(!canonical || numbers.length == 1 || numbers[numbers.length - 1] != 0, "Noncanonical VERS version");
            if (prerelease != null) {
                versionText(prerelease, false);
            }
            if (optional != null) {
                versionText(optional, true);
            }
            if (build != null) {
                require(optional == null, "Optional version information must follow the build");
                int separator = build.indexOf('-');
                optional = separator < 0 ? null : build.substring(separator + 1);
                build = separator < 0 ? build : build.substring(0, separator);
                require(!build.isEmpty() || (prerelease == null && optional != null), "Empty Java build number");
                if (!build.isEmpty()) {
                    decimal(build);
                }
                if (optional != null) {
                    versionText(optional, true);
                }
            }
            if (prerelease != null && numeric(prerelease)) {
                int start = 0;
                while (start + 1 < prerelease.length() && prerelease.charAt(start) == '0') {
                    start++;
                }
                prerelease = prerelease.substring(start);
            }
            pre = prerelease;
        }

        /// Compares numeric components and then prerelease identifiers.
        @Override
        public int compareTo(Version other) {
            for (int i = 0; i < Math.max(numbers.length, other.numbers.length); i++) {
                int order = Integer.compare(i < numbers.length ? numbers[i] : 0, i < other.numbers.length ? other.numbers[i] : 0);
                if (order != 0) {
                    return order;
                }
            }
            if (pre == null || other.pre == null) {
                return pre == null ? other.pre == null ? 0 : 1 : -1;
            }
            boolean numeric = numeric(pre);
            boolean otherNumeric = numeric(other.pre);
            if (numeric && otherNumeric) {
                int length = Integer.compare(pre.length(), other.pre.length());
                return length != 0 ? length : pre.compareTo(other.pre);
            }
            return numeric != otherNumeric ? numeric ? -1 : 1 : pre.compareTo(other.pre);
        }
    }

    /// Tests a nonempty ASCII decimal sequence.
    private static boolean numeric(String value) {
        if (value.isEmpty()) {
            return false;
        }
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
            if (ch < '0' || ch > '9') {
                return false;
            }
        }
        return true;
    }

    /// Parses a minimally written nonnegative integer in the Runtime.Version range.
    private static int decimal(String value) throws IOException {
        require(numeric(value) && (value.length() == 1 || value.charAt(0) != '0'), "Invalid Java version number");
        int result = 0;
        for (int index = 0; index < value.length(); index++) {
            int digit = value.charAt(index) - '0';
            require(result <= (Integer.MAX_VALUE - digit) / 10, "Java version number overflow");
            result = result * 10 + digit;
        }
        return result;
    }

    /// Checks a nonempty ASCII prerelease or optional-information component.
    private static void versionText(String value, boolean optional) throws IOException {
        require(!value.isEmpty(), "Empty Java version component");
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
            require(ch >= '0' && ch <= '9' || ch >= 'a' && ch <= 'z' || ch >= 'A' && ch <= 'Z'
                    || optional && (ch == '.' || ch == '-'), "Invalid Java version component");
        }
    }
}
