// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.IOException;
import java.util.Objects;

import org.glavo.janex.reader.internal.Input;

import static org.glavo.janex.reader.internal.Input.require;

/// Checks ordinary Java class-file framing and references used by the Janex CLASSFILE codec.
///
/// Checks include constant-pool slots, Modified UTF-8, attribute boundaries, method code ranges,
/// and module descriptors. Bytecode type verification remains the Java runtime's responsibility.
public final class ClassFile {
    /// Restores exact class bytes from a CLASSFILE transform using the default read policy.
    /// Pooled Modified UTF-8 bytes are copied without transcoding.
    ///
    /// @param bytes transformed input, not modified
    /// @param pool selected root or file data pool, not modified
    /// @param length required decoded byte length
    /// @return a new structurally validated ordinary class file
    /// @throws IOException if framing, references, output length, or read limits are invalid
    public static byte[] restore(byte[] bytes, byte[][] pool, int length) throws IOException {
        return restore(bytes, pool, length, ReadLimits.DEFAULT);
    }

    /// Restores and validates a CLASSFILE transform under the supplied read policy.
    /// Pooled Modified UTF-8 bytes are copied without transcoding.
    ///
    /// @param bytes transformed input, not modified
    /// @param pool selected root or file data pool, not modified
    /// @param length required decoded byte length
    /// @param limits inherited buffered-byte and constant-pool limits
    /// @return a new structurally validated ordinary class file
    /// @throws IOException if framing, references, output length, or read limits are invalid
    public static byte[] restore(byte[] bytes, byte[][] pool, int length, ReadLimits limits) throws IOException {
        return org.glavo.janex.reader.internal.codec.ClassFiles.restore(bytes, pool, length, limits);
    }

    /// Prevents instantiation.
    private ClassFile() {
    }

    /// Validates one complete ordinary class file under the default reader limits.
    ///
    /// @param bytes nonnull class bytes, unchanged for the duration of this call
    /// @throws IOException if framing, checked references, or reader limits are invalid
    public static void validate(byte[] bytes) throws IOException {
        validate(bytes, ReadLimits.DEFAULT);
    }

    /// Validates one complete ordinary class file without retaining or modifying the input.
    ///
    /// @param bytes nonnull class bytes, unchanged for the duration of this call
    /// @param limits nonnull byte and constant-pool element limits
    /// @throws IOException if framing, checked references, or reader limits are invalid
    public static void validate(byte[] bytes, ReadLimits limits) throws IOException {
        new Parser(Objects.requireNonNull(bytes), Objects.requireNonNull(limits)).validate();
    }

    /// Identifies legal locations for the attributes interpreted by this validator.
    private enum Context {
        /// Class-level attributes.
        CLASS,
        /// Field attributes.
        FIELD,
        /// Method attributes.
        METHOD,
        /// Attributes nested inside Code.
        CODE
    }

    /// Retains the selected attribute state needed to validate enclosing structures.
    private static final class Attributes {
        /// Whether a Module attribute occurred.
        boolean module;
        /// Whether a ModuleMainClass attribute occurred.
        boolean mainClass;
        /// Number of bootstrap methods, or -1 if absent.
        int bootstrapCount = -1;
        /// Whether a Code attribute occurred.
        boolean code;

        /// Creates empty attribute state.
        Attributes() {
        }
    }

    /// Owns one bounded parse and its decoded constant-pool records.
    private static final class Parser {
        /// Cursor over the borrowed class bytes.
        private final Input input;
        /// Constant tags; zero denotes an unusable slot.
        private final int[] tags;
        /// Constant body offsets into the original class bytes.
        private final int[] offsets;
        /// Decoded UTF-16 strings, including unpaired surrogates where permitted.
        private final String[] strings;
        /// Class-file major version.
        private final int major;

        /// Reads the class header and allocates the bounded constant-pool tables.
        Parser(byte[] bytes, ReadLimits limits) throws IOException {
            input = new Input(bytes, limits);
            require(u32(input) == 0xcafebabeL, "Incorrect class magic");
            int minor = u16(input);
            major = u16(input);
            require(major >= 45 && (major < 56 || minor == 0 || minor == 65535), "Invalid class-file version");
            int count = limits.elements(u16(input));
            require(count != 0, "Constant-pool count must include slot zero");
            tags = new int[count];
            offsets = new int[count];
            strings = new String[count];
        }

        /// Validates all constant records and the complete class body.
        void validate() throws IOException {
            for (int index = 1; index < tags.length; index++) {
                int tag = input.u8();
                tags[index] = tag;
                offsets[index] = input.position();
                switch (tag) {
                    case 1:
                        strings[index] = modified(input.take(u16(input)));
                        break;
                    case 3:
                    case 4:
                    case 9:
                    case 10:
                    case 11:
                    case 12:
                    case 17:
                    case 18:
                        input.take(4);
                        break;
                    case 5:
                    case 6:
                        input.take(8);
                        require(++index < tags.length, "Long or double lacks its reserved slot");
                        break;
                    case 7:
                    case 8:
                    case 16:
                    case 19:
                    case 20:
                        input.take(2);
                        break;
                    case 15:
                        input.take(3);
                        break;
                    default:
                        throw new Input.Invalid("Unknown class constant tag");
                }
            }
            constants();
            int flags = u16(input);
            String name = indirect(u16(input), 7);
            int superclass = u16(input);
            if (superclass != 0) {
                constant(superclass, 7);
            } else {
                require(name.equals("java/lang/Object") || (flags & 0x8000) != 0, "Missing superclass");
            }
            int interfaces = u16(input);
            for (int index = 0; index < interfaces; index++) {
                constant(u16(input), 7);
            }
            int fields = members(false);
            int methods = members(true);
            Attributes attributes = attributes(input, Context.CLASS);
            input.end();
            require(!attributes.mainClass || attributes.module, "ModuleMainClass requires a module descriptor");
            if ((flags & 0x8000) != 0) {
                require(major >= 53 && name.equals("module-info") && superclass == 0 && interfaces == 0
                        && fields == 0 && methods == 0 && attributes.module, "Invalid module-info class structure");
            } else {
                require(!attributes.module, "Module attribute requires ACC_MODULE");
            }
            for (int index = 1; index < tags.length; index++) {
                if (tags[index] == 17 || tags[index] == 18) {
                    require(bodyU16(index, 0) < attributes.bootstrapCount, "Missing bootstrap method");
                }
            }
        }

        /// Checks constant-pool reference types and tag availability in the recorded class version.
        private void constants() throws IOException {
            for (int index = 1; index < tags.length; index++) {
                int tag = tags[index];
                switch (tag) {
                    case 7:
                    case 8:
                    case 16:
                    case 19:
                    case 20:
                        constant(bodyU16(index, 0), 1);
                        break;
                    case 9:
                    case 10:
                    case 11:
                        constant(bodyU16(index, 0), 7);
                        constant(bodyU16(index, 2), 12);
                        break;
                    case 12:
                        constant(bodyU16(index, 0), 1);
                        constant(bodyU16(index, 2), 1);
                        break;
                    case 17:
                    case 18:
                        constant(bodyU16(index, 2), 12);
                        break;
                    case 15:
                        int kind = input.byteAt(offsets[index]);
                        int reference = bodyU16(index, 1);
                        if (kind >= 1 && kind <= 4) {
                            constant(reference, 9);
                        } else if (kind == 5 || kind == 8) {
                            constant(reference, 10);
                        } else if (kind == 6 || kind == 7) {
                            constant(reference, 10, major >= 52 ? 11 : 10);
                        } else {
                            require(kind == 9, "Invalid method-handle kind");
                            constant(reference, 11);
                        }
                        break;
                    default:
                        break;
                }
                require(!((tag == 15 || tag == 16 || tag == 18) && major < 51)
                        && !((tag == 19 || tag == 20) && major < 53) && !(tag == 17 && major < 55),
                        "Constant tag is unavailable in this class-file version");
            }
        }

        /// Reads an integer inside a previously bounded constant body.
        private int bodyU16(int index, int offset) {
            int start = offsets[index] + offset;
            return input.byteAt(start) << 8 | input.byteAt(start + 1);
        }

        /// Requires a usable constant-pool slot with one of the supplied tags.
        private void constant(int index, int... allowed) throws IOException {
            require(index > 0 && index < tags.length, "Invalid class constant-pool reference");
            for (int tag : allowed) {
                if (tags[index] == tag) {
                    return;
                }
            }
            throw new Input.Invalid("Invalid class constant-pool reference type");
        }

        /// Reads a UTF-8 constant as Unicode scalar values for interpreted identifiers.
        private String text(int index) throws IOException {
            constant(index, 1);
            String value = strings[index];
            for (int offset = 0; offset < value.length(); offset++) {
                char unit = value.charAt(offset);
                if (Character.isHighSurrogate(unit)) {
                    require(++offset < value.length() && Character.isLowSurrogate(value.charAt(offset)), "Unpaired class identifier surrogate");
                } else {
                    require(!Character.isLowSurrogate(unit), "Unpaired class identifier surrogate");
                }
            }
            return value;
        }

        /// Resolves a Class, Module, or Package constant to its text.
        private String indirect(int index, int tag) throws IOException {
            constant(index, tag);
            return text(bodyU16(index, 0));
        }

        /// Validates an optional UTF-8 reference, with zero denoting absence.
        private void optionalText(int index) throws IOException {
            if (index != 0) {
                text(index);
            }
        }

        /// Reads fields or methods and checks Code presence against method flags.
        private int members(boolean methods) throws IOException {
            int count = u16(input);
            for (int index = 0; index < count; index++) {
                int flags = u16(input);
                constant(u16(input), 1);
                constant(u16(input), 1);
                Attributes attributes = attributes(input, methods ? Context.METHOD : Context.FIELD);
                require(!methods || attributes.code == ((flags & 0x0500) == 0), "Method Code disagrees with native or abstract flags");
            }
            return count;
        }

        /// Checks attribute lengths, interpreting only the codec's structural attributes.
        private Attributes attributes(Input cursor, Context context) throws IOException {
            Attributes result = new Attributes();
            int count = u16(cursor);
            for (int index = 0; index < count; index++) {
                String name = text(u16(cursor));
                Input value = new Input(cursor.take(u32(cursor)), cursor.limits());
                switch (name) {
                    case "Module":
                        require(context == Context.CLASS && !result.module, "Misplaced or duplicate Module attribute");
                        result.module = true;
                        module(value);
                        break;
                    case "ModuleMainClass":
                        require(context == Context.CLASS && !result.mainClass, "Misplaced or duplicate ModuleMainClass attribute");
                        result.mainClass = true;
                        indirect(u16(value), 7);
                        break;
                    case "BootstrapMethods":
                        require(context == Context.CLASS && result.bootstrapCount < 0, "Misplaced or duplicate BootstrapMethods attribute");
                        result.bootstrapCount = u16(value);
                        for (int bootstrap = 0; bootstrap < result.bootstrapCount; bootstrap++) {
                            constant(u16(value), 15);
                            int arguments = u16(value);
                            for (int argument = 0; argument < arguments; argument++) {
                                constant(u16(value), 3, 4, 5, 6, 7, 8, 15, 16, 17);
                            }
                        }
                        break;
                    case "Code":
                        require(context == Context.METHOD && !result.code, "Misplaced or duplicate Code attribute");
                        result.code = true;
                        value.take(4);
                        long length = u32(value);
                        require(length >= 1 && length < 65536, "Invalid method code length");
                        value.take(length);
                        int handlers = u16(value);
                        for (int handler = 0; handler < handlers; handler++) {
                            int start = u16(value);
                            int end = u16(value);
                            int target = u16(value);
                            int caught = u16(value);
                            require(start < end && end <= length && target < length, "Invalid exception-handler range");
                            if (caught != 0) {
                                constant(caught, 7);
                            }
                        }
                        attributes(value, Context.CODE);
                        break;
                    default:
                        value.skip(value.remaining());
                        break;
                }
                value.end();
            }
            return result;
        }

        /// Checks a Module attribute and every constant-pool reference it contains.
        private void module(Input cursor) throws IOException {
            indirect(u16(cursor), 19);
            cursor.take(2);
            optionalText(u16(cursor));
            int count = u16(cursor);
            for (int index = 0; index < count; index++) {
                indirect(u16(cursor), 19);
                cursor.take(2);
                optionalText(u16(cursor));
            }
            for (int kind = 0; kind < 2; kind++) {
                count = u16(cursor);
                for (int index = 0; index < count; index++) {
                    constant(u16(cursor), 20);
                    cursor.take(2);
                    int targets = u16(cursor);
                    for (int target = 0; target < targets; target++) {
                        constant(u16(cursor), 19);
                    }
                }
            }
            count = u16(cursor);
            for (int index = 0; index < count; index++) {
                constant(u16(cursor), 7);
            }
            count = u16(cursor);
            for (int index = 0; index < count; index++) {
                constant(u16(cursor), 7);
                int providers = u16(cursor);
                require(providers != 0, "Empty module provider list");
                for (int provider = 0; provider < providers; provider++) {
                    constant(u16(cursor), 7);
                }
            }
        }
    }

    /// Decodes canonical Modified UTF-8 while preserving unpaired UTF-16 surrogates.
    private static String modified(byte[] bytes) throws IOException {
        StringBuilder value = new StringBuilder(bytes.length);
        for (int index = 0; index < bytes.length;) {
            int first = bytes[index++] & 255;
            if (first >= 1 && first <= 127) {
                value.append((char) first);
                continue;
            }
            int count = first >= 0xc0 && first <= 0xdf ? 2 : first >= 0xe0 && first <= 0xef ? 3 : 0;
            require(count != 0 && bytes.length - index >= count - 1, "Invalid Modified UTF-8 leading byte or length");
            int unit = first & (count == 2 ? 31 : 15);
            for (int part = 1; part < count; part++) {
                int next = bytes[index++] & 255;
                require((next & 0xc0) == 0x80, "Invalid Modified UTF-8 continuation");
                unit = unit << 6 | next & 63;
            }
            require(!(count == 2 && unit != 0 && unit < 128) && !(count == 3 && unit < 2048), "Overlong Modified UTF-8");
            value.append((char) unit);
        }
        return value.toString();
    }

    /// Reads an unsigned big-endian class-file short.
    private static int u16(Input cursor) throws IOException {
        return cursor.u8() << 8 | cursor.u8();
    }

    /// Reads an unsigned big-endian class-file integer.
    private static long u32(Input cursor) throws IOException {
        return (long) u16(cursor) << 16 | u16(cursor);
    }
}
