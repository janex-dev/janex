// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.IOException;
import java.math.BigInteger;
import java.util.*;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.janex.format.Input.*;

/// Evaluates conditions against the current Java process and the run invocation channel.
final class Conditions {
    /// Prevents instantiation.
    private Conditions() {
    }

    /// Validates a condition and returns whether it matches this runtime.
    static boolean matches(Map<Object, Object> condition) throws IOException {
        integers(condition);
        require(!has(condition, 0) && !has(condition, 3), "Reserved condition key");
        String os = System.getProperty("os.name");
        os = os.startsWith("Windows") ? "windows" : os.equals("Mac OS X") ? "macos" : os.equals("Linux") ? "linux" : os;
        String arch = System.getProperty("os.arch");
        arch = arch.equals("amd64") || arch.equals("x86_64") ? "x86-64"
                : arch.matches("i[3-6]86") ? "x86" : arch.equals("arm64") ? "aarch64" : arch;
        boolean result = true;
        if (has(condition, 1)) {
            result &= selector(get(condition, 1), os);
        }
        if (has(condition, 2)) {
            result &= selector(get(condition, 2), arch);
        }
        if (has(condition, 4)) {
            result &= selector(get(condition, 4), "run");
        }
        if (has(condition, 5)) {
            Map<Object, Object> runtime = integers(map(get(condition, 5)));
            String type = nonempty(get(runtime, 0));
            Map<Object, Object> requirements = integers(map(get(runtime, 1)));
            if (type.equals("janex.java")) {
                if (has(requirements, 0)) {
                    result &= range(nonempty(get(requirements, 0)), System.getProperty("java.version"));
                }
                if (has(requirements, 1)) {
                    result &= nonempty(get(requirements, 1)).equals(System.getProperty("java.vendor"));
                }
            } else {
                result = false;
            }
        }
        return result;
    }

    /// Requires a nonempty CBOR text string.
    static String nonempty(Object value) throws IOException {
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
        return new Version(System.getProperty("java.version"), false).numbers[0];
    }

    /// Validates a canonical VERS timeline and tests one Java version.
    static boolean range(String text, String candidate) throws IOException {
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
        Version current = new Version(candidate, false);
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
            if (value.matches("8u[0-9]+")) {
                value = "8.0." + value.substring(2);
                canonical = false;
            } else if (value.startsWith("1.8.0")) {
                Matcher alias = Pattern.compile("1\\.8\\.0(?:_([0-9]+))?(?:-([A-Za-z0-9]+))?").matcher(value);
                require(alias.matches(), "Invalid Java 8 alias");
                String update = alias.group(1);
                String suffix = alias.group(2);
                if (update != null && suffix != null && suffix.matches("b[0-9]+")) {
                    String build = suffix.substring(1);
                    require(build.length() == 1 || !build.startsWith("0"), "Nonminimal Java 8 build");
                    require(new BigInteger(build).bitLength() <= 31, "Java 8 build overflow");
                    suffix = null;
                }
                value = update == null ? "8" : "8.0." + update;
                if (suffix != null) {
                    value += "-" + suffix;
                }
                canonical = false;
            }
            Matcher matcher = Pattern.compile("([0-9]+(?:\\.[0-9]+)*)(?:-([A-Za-z0-9]+))?(?:\\+([0-9]*))?(?:-([A-Za-z0-9.-]+))?").matcher(value);
            require(matcher.matches(), "Invalid Java version");
            String[] components = matcher.group(1).split("\\.");
            numbers = new int[components.length];
            for (int i = 0; i < numbers.length; i++) {
                String component = components[i];
                require(component.length() == 1 || !component.startsWith("0"), "Nonminimal Java version");
                BigInteger integer = new BigInteger(component);
                require(integer.bitLength() <= 31, "Java version number overflow");
                numbers[i] = integer.intValue();
            }
            require(numbers[0] >= 8, "Java 8 or later is required");
            require(!canonical || numbers.length == 1 || numbers[numbers.length - 1] != 0, "Noncanonical VERS version");
            pre = matcher.group(2);
            String build = matcher.group(3);
            if (build != null) {
                require(!build.isEmpty() || (pre == null && matcher.group(4) != null), "Empty Java build number");
                require(build.length() <= 1 || !build.startsWith("0"), "Nonminimal Java build number");
                if (!build.isEmpty()) {
                    require(new BigInteger(build).bitLength() <= 31, "Java build number overflow");
                }
            }
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
            boolean numeric = pre.matches("[0-9]+");
            boolean otherNumeric = other.pre.matches("[0-9]+");
            if (numeric && otherNumeric) {
                return new BigInteger(pre).compareTo(new BigInteger(other.pre));
            }
            return numeric != otherNumeric ? numeric ? -1 : 1 : pre.compareTo(other.pre);
        }
    }
}
