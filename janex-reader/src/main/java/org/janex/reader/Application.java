// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader;

import java.io.IOException;
import java.util.*;

import org.janex.reader.internal.Conditions;
import org.janex.reader.internal.Input;

import static org.janex.reader.internal.Input.*;

/// Retains a validated application descriptor and its localized presentation metadata.
///
/// Unknown application types retain an opaque integer-keyed descriptor and cannot be launched
/// by the Java launcher. Construction does not resolve icon blobs or other resources. Instances
/// own their encoded bytes and remain usable after the source reader is closed.
public final class Application {
    /// Original complete application section bytes.
    private final byte[] encoded;
    /// Original type-info CBOR map.
    private final byte[] typeInfo;
    /// File-local application identifier.
    private final String id;
    /// Descriptor schema identifier.
    private final String type;
    /// Optional localized display name.
    private final LocalizedText name;
    /// Optional localized description, whose text may be empty.
    private final LocalizedText comment;
    /// Optional application version text.
    private final String version;
    /// Optional integration command used as a title fallback.
    private final String command;
    /// Whether the descriptor requests a windowed launch.
    private final boolean windowed;
    /// Validated descriptor map used by the internal launch selector.
    final Map<Object, Object> descriptor;

    /// Decodes an application section and validates all supported fields and configuration branches.
    ///
    /// @param encoded complete section including JANEXAPP magic and its Sized CBOR body
    /// @param typeInfo deterministic CBOR type-info map, excluding a Sized prefix
    /// @throws IOException if encoding, presentation, or a supported descriptor schema is invalid
    public Application(byte[] encoded, byte[] typeInfo) throws IOException {
        this(encoded, typeInfo, ReadLimits.DEFAULT);
    }

    /// Decodes an application with explicit allocation, collection, and nesting limits.
    ///
    /// @param encoded complete section including JANEXAPP magic and its Sized CBOR body
    /// @param typeInfo deterministic CBOR type-info map, excluding a Sized prefix
    /// @param limits nonnull policy applied before copying bytes and throughout nested decoding
    /// @throws IOException if encoding, supported metadata, or a resource limit is invalid
    public Application(byte[] encoded, byte[] typeInfo, ReadLimits limits) throws IOException {
        Objects.requireNonNull(limits).bytes(encoded.length);
        limits.bytes(typeInfo.length);
        this.encoded = encoded.clone();
        this.typeInfo = typeInfo.clone();
        Input infoInput = new Input(this.typeInfo, limits);
        Map<Object, Object> info = integers(map(infoInput.cbor(0)));
        infoInput.end();
        id = Conditions.nonempty(get(info, 0));
        type = Conditions.nonempty(get(info, 1));
        Input input = new Input(this.encoded, limits);
        require(input.little(8) == 0x50504158454e414aL, "Incorrect application magic");
        Map<Object, Object> value = integers(input.map());
        input.end();
        descriptor = integers(map(get(value, 0)));
        name = has(value, 1) ? new LocalizedText(get(value, 1), true) : null;
        comment = has(value, 3) ? new LocalizedText(get(value, 3), false) : null;
        version = has(value, 2) ? Conditions.nonempty(get(value, 2)) : null;
        String command = null;
        if (has(value, 4)) {
            Map<Object, Object> integration = integers(map(get(value, 4)));
            if (has(integration, 0)) {
                command = Conditions.nonempty(get(integration, 0));
                require(!command.equals(".") && !command.equals("..") && command.indexOf('/') < 0
                        && command.indexOf(0) < 0, "Application command must be a single filename");
            }
            if (has(integration, 1)) {
                require(get(integration, 1) instanceof Boolean, "Invalid desktop integration request");
            }
            if (has(integration, 2)) {
                List<Object> icons = list(get(integration, 2));
                require(!icons.isEmpty(), "Empty application icon array");
                for (Object item : icons) {
                    Map<Object, Object> icon = integers(map(item));
                    Conditions.nonempty(get(icon, 0));
                    List<Object> reference = list(get(icon, 1));
                    require(reference.size() == 2, "Invalid icon BlobRef");
                    number(reference.get(0));
                    number(reference.get(1));
                }
            }
        }
        this.command = command;
        long mode = has(value, 5) ? number(get(value, 5)) : 0;
        require(mode == 0 || mode == 1, "Invalid application launch mode");
        windowed = mode == 1;
        if (type.equals("janex.java")) {
            JanexReader.validateConfiguration(integers(map(get(descriptor, 0))), limits);
        }
    }

    /// Returns the file-local application identifier.
    public String id() {
        return id;
    }

    /// Returns the descriptor schema identifier.
    public String type() {
        return type;
    }

    /// Returns the application's version, or null when omitted.
    public String version() {
        return version;
    }

    /// Returns whether the descriptor requests a windowed launch.
    public boolean windowed() {
        return windowed;
    }

    /// Returns a localized title, falling back to the command and then application ID.
    ///
    /// @param locale language tag; malformed tags use the translation fallback
    /// @return selected nonempty title
    public String title(String locale) {
        Objects.requireNonNull(locale);
        return name == null ? command == null ? id : command : name.select(locale);
    }

    /// Returns a localized description, or null when omitted; empty text remains present.
    ///
    /// @param locale language tag; malformed tags use the translation fallback
    /// @return selected text, possibly empty, or null
    public String comment(String locale) {
        Objects.requireNonNull(locale);
        return comment == null ? null : comment.select(locale);
    }

    /// Returns an owned copy of the exact original section, preserving all unknown fields.
    public byte[] encoded() {
        return encoded.clone();
    }

    /// Returns an owned copy of the exact original type-info CBOR map.
    public byte[] typeInfo() {
        return typeInfo.clone();
    }
}
