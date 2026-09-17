// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal;

import java.util.*;
import java.util.function.BiConsumer;
import java.util.function.Function;

/// An immutable resource-name map backed by parallel arrays and an integer hash index.
/// Iteration follows the supplied array order. Keys and values must be non-null.
/// Values are retained by reference; their own mutability is unaffected.
///
/// @param <V> resource descriptor type
public final class ResourceTable<V> extends AbstractMap<String, V> {
    /// Resource names in iteration order, shared by mapped tables.
    private final String[] names;
    /// Descriptors corresponding to the name array.
    private final Object[] values;
    /// One-based entry numbers in an open-addressed table; zero denotes an empty slot.
    private final int[] index;

    /// Takes ownership of parallel arrays and builds their name index.
    /// The caller must not modify either array after this call.
    ///
    /// @param names distinct, non-null resource names in iteration order
    /// @param values non-null descriptors, one per name
    /// @throws IllegalArgumentException if lengths differ, names repeat, or there are more than
    ///     2^29 entries
    /// @throws NullPointerException if an array, name, or descriptor is null
    public ResourceTable(String[] names, V[] values) {
        if (names.length != values.length || names.length > (1 << 29)) {
            throw new IllegalArgumentException("Invalid resource table size");
        }
        this.names = names;
        this.values = values;
        int capacity = 1;
        while (capacity < names.length * 2) capacity <<= 1;
        index = new int[capacity];
        for (int i = 0; i < names.length; i++) {
            Objects.requireNonNull(values[i]);
            String name = Objects.requireNonNull(names[i]);
            int slot = slot(name, index.length - 1);
            while (index[slot] != 0) {
                if (names[index[slot] - 1].equals(name)) {
                    throw new IllegalArgumentException("Duplicate resource name: " + name);
                }
                slot = (slot + 1) & (index.length - 1);
            }
            index[slot] = i + 1;
        }
    }

    /// Shares an existing immutable name index and takes ownership of mapped values.
    private ResourceTable(String[] names, Object[] values, int[] index) {
        this.names = names;
        this.values = values;
        this.index = index;
    }

    /// Maps descriptors in iteration order, sharing the name index when the input is a table.
    /// Each descriptor is mapped once. Other maps are snapshotted in their iteration order.
    ///
    /// @param source non-null map with non-null keys and values; must remain unchanged during this call
    /// @param mapper non-null mapping function that must return non-null descriptors
    /// @param <S> source descriptor type
    /// @param <T> result descriptor type
    /// @return immutable table with the same names and iteration order
    /// @throws NullPointerException if a required value is null
    public static <S, T> ResourceTable<T> mapValues(Map<String, S> source, Function<? super S, ? extends T> mapper) {
        Objects.requireNonNull(mapper);
        Object[] values = new Object[source.size()];
        if (source instanceof ResourceTable) {
            ResourceTable<S> table = (ResourceTable<S>) source;
            for (int i = 0; i < values.length; i++) {
                values[i] = Objects.requireNonNull(mapper.apply(table.value(i)));
            }
            return new ResourceTable<T>(table.names, values, table.index);
        }
        String[] names = new String[values.length];
        int i = 0;
        for (Entry<String, S> entry : source.entrySet()) {
            names[i] = entry.getKey();
            values[i++] = Objects.requireNonNull(mapper.apply(Objects.requireNonNull(entry.getValue())));
        }
        ResourceTable<Object> table = new ResourceTable<Object>(names, values);
        return new ResourceTable<T>(names, values, table.index);
    }

    /// Spreads a cached string hash across the power-of-two table.
    private static int slot(String name, int mask) {
        int hash = name.hashCode();
        return (hash ^ (hash >>> 16)) & mask;
    }

    /// Returns a descriptor at a valid entry position.
    @SuppressWarnings("unchecked")
    private V value(int position) {
        return (V) values[position];
    }

    @Override
    public int size() {
        return names.length;
    }

    /// Returns the named descriptor, or null for an absent or non-string key.
    @Override
    public V get(Object key) {
        if (!(key instanceof String)) return null;
        int slot = slot((String) key, index.length - 1);
        for (int entry; (entry = index[slot]) != 0; slot = (slot + 1) & (index.length - 1)) {
            if (names[entry - 1].equals(key)) return value(entry - 1);
        }
        return null;
    }

    @Override
    public boolean containsKey(Object key) {
        return get(key) != null;
    }

    @Override
    public void forEach(BiConsumer<? super String, ? super V> action) {
        Objects.requireNonNull(action);
        for (int i = 0; i < names.length; i++) action.accept(names[i], value(i));
    }

    /// Returns an immutable view of the resource names in iteration order.
    @Override
    public Set<String> keySet() {
        return Collections.unmodifiableSet(new AbstractSet<String>() {
            @Override
            public int size() {
                return names.length;
            }

            @Override
            public boolean contains(Object key) {
                return containsKey(key);
            }

            @Override
            public Iterator<String> iterator() {
                return Arrays.asList(names).iterator();
            }
        });
    }

    /// Returns an immutable view whose entries cannot replace descriptors.
    @Override
    public Set<Entry<String, V>> entrySet() {
        return Collections.unmodifiableSet(new AbstractSet<Entry<String, V>>() {
            @Override
            public int size() {
                return names.length;
            }

            @Override
            public Iterator<Entry<String, V>> iterator() {
                return new Iterator<Entry<String, V>>() {
                    /// Position of the next entry in traversal order.
                    private int position;

                    @Override
                    public boolean hasNext() {
                        return position < names.length;
                    }

                    @Override
                    public Entry<String, V> next() {
                        if (!hasNext()) throw new NoSuchElementException();
                        int current = position++;
                        return new SimpleImmutableEntry<String, V>(names[current], value(current));
                    }
                };
            }
        });
    }
}
