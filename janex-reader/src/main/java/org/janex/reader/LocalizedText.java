// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader;

import java.io.IOException;
import java.util.*;

import static org.janex.reader.internal.Input.*;

/// Validates display translations and selects text using BCP 47 language lookup.
final class LocalizedText {
    /// Bare text, or null for a translation map.
    private final String text;
    /// Well-formed lowercase tags in original deterministic map order.
    private final Map<String, String> translations = new LinkedHashMap<String, String>();
    /// Grandfathered language tags accepted independently of the normal subtag grammar.
    private static final Set<String> GRANDFATHERED = new HashSet<String>(Arrays.asList(
            "art-lojban", "cel-gaulish", "en-gb-oed", "i-ami", "i-bnn", "i-default", "i-enochian",
            "i-hak", "i-klingon", "i-lux", "i-mingo", "i-navajo", "i-pwn", "i-tao", "i-tay", "i-tsu",
            "no-bok", "no-nyn", "sgn-be-fr", "sgn-be-nl", "sgn-ch-de", "zh-guoyu", "zh-hakka",
            "zh-min", "zh-min-nan", "zh-xiang"));

    /// Checks text values and usable tag uniqueness without requiring registry membership.
    LocalizedText(Object value, boolean nonempty) throws IOException {
        text = value instanceof String ? text(value) : null;
        if (text != null) {
            require(!nonempty || !text.isEmpty(), "Empty localized display name");
            return;
        }
        for (Map.Entry<Object, Object> entry : map(value).entrySet()) {
            String tag = text(entry.getKey());
            String translation = text(entry.getValue());
            require(!nonempty || !translation.isEmpty(), "Empty localized display name");
            if (wellFormed(tag)) {
                require(translations.put(tag.toLowerCase(Locale.ROOT), translation) == null, "Duplicate language tag");
            }
        }
        require(!translations.isEmpty(), "Localized text has no well-formed language tag");
    }

    /// Selects a translation by lookup, then und, then original deterministic map order.
    String select(String locale) {
        Objects.requireNonNull(locale);
        if (text != null) {
            return text;
        }
        if (wellFormed(locale)) {
            String range = locale.toLowerCase(Locale.ROOT);
            while (!range.isEmpty()) {
                for (Map.Entry<String, String> entry : translations.entrySet()) {
                    if (entry.getKey().equals(range) || entry.getKey().startsWith(range + "-")) {
                        return entry.getValue();
                    }
                }
                int end = range.lastIndexOf('-');
                if (end < 0) {
                    break;
                }
                range = range.substring(0, end);
                if (range.length() - range.lastIndexOf('-') == 2) {
                    int singleton = range.lastIndexOf('-');
                    range = singleton < 0 ? "" : range.substring(0, singleton);
                }
            }
        }
        return translations.containsKey("und") ? translations.get("und") : translations.values().iterator().next();
    }

    /// Checks the BCP 47 subtag grammar without registry, preferred-value, or duplicate-subtag validation.
    private static boolean wellFormed(String tag) {
        for (int i = 0; i < tag.length(); i++) {
            if (tag.charAt(i) > 127) {
                return false;
            }
        }
        String[] parts = tag.toLowerCase(Locale.ROOT).split("-", -1);
        for (String part : parts) {
            if (part.isEmpty() || part.length() > 8 || !characters(part, false)) {
                return false;
            }
        }
        if (GRANDFATHERED.contains(tag.toLowerCase(Locale.ROOT))) {
            return true;
        }
        if (parts[0].equals("x")) {
            return parts.length > 1;
        }
        if (parts[0].length() < 2 || !characters(parts[0], true)) {
            return false;
        }
        int index = 1;
        if (parts[0].length() <= 3) {
            for (int count = 0; count < 3 && index < parts.length && parts[index].length() == 3
                    && characters(parts[index], true); count++) {
                index++;
            }
        }
        if (index < parts.length && parts[index].length() == 4 && characters(parts[index], true)) {
            index++;
        }
        if (index < parts.length && (parts[index].length() == 2 && characters(parts[index], true)
                || parts[index].matches("[0-9]{3}"))) {
            index++;
        }
        while (index < parts.length && (parts[index].length() >= 5
                || parts[index].length() == 4 && Character.isDigit(parts[index].charAt(0)))) {
            index++;
        }
        while (index < parts.length && parts[index].length() == 1 && !parts[index].equals("x")) {
            int start = ++index;
            while (index < parts.length && parts[index].length() >= 2) {
                index++;
            }
            if (index == start) {
                return false;
            }
        }
        if (index < parts.length && parts[index].equals("x")) {
            return index + 1 < parts.length;
        }
        return index == parts.length;
    }

    /// Checks lowercase ASCII letters and, when permitted, digits.
    private static boolean characters(String part, boolean alphabetic) {
        for (int i = 0; i < part.length(); i++) {
            char ch = part.charAt(i);
            if (!(ch >= 'a' && ch <= 'z') && (alphabetic || !(ch >= '0' && ch <= '9'))) {
                return false;
            }
        }
        return true;
    }
}
