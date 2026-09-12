// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.IOException;
import java.math.BigInteger;
import java.util.Arrays;
import java.util.Map;

/// Exercises deterministic encoding and version boundaries independently of the native writer.
public final class ReaderTest {
    /// Prevents instantiation.
    private ReaderTest() {
    }

    /// Runs boundary checks without external test dependencies.
    ///
    /// @param arguments unused
    /// @throws Exception if an accepted value fails or a malformed value is accepted
    public static void main(String[] arguments) throws Exception {
        check(new Input(new byte[]{0}).map().isEmpty());
        reject(() -> new Input(new byte[]{1, (byte) 0xa0}).map());
        reject(() -> new Input(new byte[]{3, (byte) 0xa1, 0, 0, 0}).end());
        reject(() -> new Input(new byte[]{4, (byte) 0xa1, 0x18, 0, 0}).map());
        reject(() -> new Input(new byte[]{5, (byte) 0xa2, 1, 0, 0, 0}).map());
        reject(() -> new Input(new byte[]{5, (byte) 0xa2, 0, 0, 0, 1}).map());
        reject(() -> new Input(new byte[]{4, (byte) 0xa1, 0, 0x61, (byte) 0xff}).map());
        reject(() -> new Input(new byte[]{1, (byte) 0xbf}).map());
        reject(() -> new Input(new byte[]{5, (byte) 0xa1, 0, (byte) 0xf9, 0x7e, 1}).map());
        reject(() -> new Input(new byte[]{7, (byte) 0xa1, 0, (byte) 0xfa, 0x3f, (byte) 0x80, 0, 0}).map());
        Map<Object, Object> map = new Input(new byte[]{5, (byte) 0xa1, 0, (byte) 0xf9, 0x3c, 0}).map();
        check(map.size() == 1);
        byte[] maximum = new byte[10];
        Arrays.fill(maximum, (byte) 0xff);
        maximum[9] = 1;
        check(new Input(maximum).uint() == -1L);
        maximum[9] = 2;
        reject(() -> new Input(maximum).uint());
        check(new Input(new byte[]{(byte) 0x80, 0}).uint() == 0);
        reject(() -> new Input(new byte[]{(byte) 0x80}).uint());
        byte[] unsigned = {11, (byte) 0xa1, 0, 0x1b, (byte) 0xff, (byte) 0xff, (byte) 0xff, (byte) 0xff,
                (byte) 0xff, (byte) 0xff, (byte) 0xff, (byte) 0xff};
        check(Input.number(Input.get(new Input(unsigned).map(), 0)) == -1L);
        check(Conditions.range("vers:jep322/>=8", "1.8.0_452-b9"));
        reject(() -> Conditions.range("vers:jep322/>=8", "1.8.0_452-b09"));
        reject(() -> Conditions.range("vers:jep322/>=8", "1.8.0+1"));
        reject(() -> Conditions.range("vers:jep322/>=8", "1.8.0_452-ea-extra"));
        check(Conditions.range("vers:jep322/>=11|<17|>=21", "25.0.1+8-LTS"));
        check(!Conditions.range("vers:jep322/>=11|<17|>=21", "17"));
        check(!Conditions.range("vers:jep322/>=11|!=17|<21", "17"));
        check(Conditions.range("vers:jep322/>=11|!=17|<21", "18"));
        check(!Conditions.range("vers:jep322/>=25", "25-ea"));
        check(Conditions.range("vers:jep322/8u452", "1.8.0_452"));
        reject(() -> Conditions.range("vers:jep322/>=11|>=17", "25"));
        reject(() -> Conditions.range("vers:jep322/17|<21", "25"));
        reject(() -> Conditions.range("vers:jep322/17.0", "25"));
        reject(() -> Conditions.range("vers:jep322/17|11", "25"));
        Map<Object, Object> reserved = new Input(new byte[]{3, (byte) 0xa1, 0, 0}).map();
        reject(() -> Conditions.matches(reserved));
        // Unknown extension tags remain opaque rather than being interpreted as timestamps.
        Map<Object, Object> extension = new Input(new byte[]{5, (byte) 0xa1, 0x18, 99, (byte) 0xc2, 0x40}).map();
        check(extension.containsKey(BigInteger.valueOf(99)));
    }

    /// Fails the test when an expected invariant does not hold.
    private static void check(boolean condition) {
        if (!condition) {
            throw new AssertionError();
        }
    }

    /// Requires a malformed value to fail with the reader's checked format exception.
    private static void reject(Action action) throws Exception {
        try {
            action.run();
        } catch (IOException expected) {
            return;
        }
        throw new AssertionError("Malformed input accepted");
    }

    /// One potentially failing parser operation.
    private interface Action {
        /// Executes a parser check.
        void run() throws Exception;
    }
}
