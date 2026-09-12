// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.*;
import java.net.*;
import java.security.CodeSource;
import java.security.cert.Certificate;
import java.util.*;
import java.util.jar.*;

/// Loads Host-selected classpath roots through standard parent-first delegation.
///
/// The JVM installs this as its system loader. Resource lookup retains root order and
/// duplicates, package manifest attributes, sealing, and one code-source URL per root.
public final class ResourceLoader extends URLClassLoader {
    static {
        registerAsParallelCapable();
    }

    /// Owned resource snapshot and decoded blob cache.
    private final ResourceIndex index;
    /// URL handlers and manifests in classpath order.
    private final List<Root> roots = new ArrayList<Root>();
    /// Packages defined by this loader, excluding packages belonging to parents.
    private final Map<String, Package> packages = new HashMap<String, Package>();

    /// Creates the JVM system loader using the private index in its own bootstrap JAR.
    ///
    /// @param parent the JVM's original application loader
    /// @throws IOException if the index or a runtime manifest cannot be read
    public ResourceLoader(ClassLoader parent) throws IOException {
        super(new URL[0], parent);
        InputStream data = ResourceLoader.class.getResourceAsStream("resources.bin");
        if (data == null) throw new IOException("Missing Janex resource index");
        index = new ResourceIndex(data);
        try {
            for (ResourceIndex.Root root : index.roots) roots.add(new Root(root, roots.size()));
            String handlers = System.getProperty("java.protocol.handler.pkgs", "");
            String prefix = "org.janex.bootstrap.protocol";
            if (!Arrays.asList(handlers.split("\\|", -1)).contains(prefix)) {
                System.setProperty("java.protocol.handler.pkgs", handlers.isEmpty() ? prefix : handlers + "|" + prefix);
            }
        } catch (Throwable failure) {
            try {
                index.close();
            } catch (IOException close) {
                failure.addSuppressed(close);
            }
            throw failure;
        }
    }

    /// Defines the first matching class after inherited parent delegation has failed.
    @Override
    protected Class<?> findClass(String name) throws ClassNotFoundException {
        String path = name.replace('.', '/') + ".class";
        for (Root root : roots) {
            ResourceIndex.Resource resource = root.root.files.get(path);
            if (resource == null || resource.id == -1) continue;
            try {
                byte[] bytes = resource.read();
                int dot = name.lastIndexOf('.');
                if (dot != -1) ensurePackage(name.substring(0, dot), root);
                return defineClass(name, bytes, 0, bytes.length, new CodeSource(root.base, (Certificate[]) null));
            } catch (IOException failure) {
                throw new ClassNotFoundException(name, failure);
            }
        }
        return super.findClass(name);
    }

    /// Defines manifest package metadata and checks sealing against the original root.
    private void ensurePackage(String name, Root root) {
        synchronized (packages) {
            Package defined = packages.get(name);
            if (defined == null) {
                defined = root.manifest == null
                        ? definePackage(name, null, null, null, null, null, null, null)
                        : definePackage(name, root.manifest, root.base);
                packages.put(name, defined);
            } else if (defined.isSealed() ? !defined.isSealed(root.base) : root.sealed(name)) {
                throw new SecurityException("Sealing violation: " + name);
            }
        }
    }

    /// Returns the first resource in local classpath order, followed by agent-appended JARs.
    @Override
    public URL findResource(String name) {
        for (Root root : roots) if (root.root.files.containsKey(name)) return root.url(name);
        return super.findResource(name);
    }

    /// Enumerates all local matches without collapsing equal resource names across roots.
    @Override
    public Enumeration<URL> findResources(String name) throws IOException {
        List<URL> matches = new ArrayList<URL>();
        for (Root root : roots) if (root.root.files.containsKey(name)) matches.add(root.url(name));
        Enumeration<URL> appended = super.findResources(name);
        while (appended.hasMoreElements()) matches.add(appended.nextElement());
        return Collections.enumeration(matches);
    }

    /// Accepts instrumentation's append request using the standard system-loader hook.
    ///
    /// @param path local JAR path supplied by the JVM
    private void appendToClassPathForInstrumentation(String path) throws IOException {
        addURL(new File(path).toURI().toURL());
    }

    /// Closes appended JARs and the resource snapshot; already loaded classes remain usable.
    @Override
    public void close() throws IOException {
        try {
            super.close();
        } finally {
            index.close();
        }
    }

    /// Opens a reconstructed `janex:` URL through the active system resource loader.
    ///
    /// @param url absolute resource URL generated for this launch
    /// @return a connection to the named resource
    /// @throws IOException if no resource loader is active or the resource is absent
    public static URLConnection connect(URL url) throws IOException {
        ClassLoader loader = ClassLoader.getSystemClassLoader();
        if (loader instanceof ResourceLoader) {
            for (Root root : ((ResourceLoader) loader).roots) {
                try {
                    return root.openConnection(url);
                } catch (FileNotFoundException absent) { /* Continue with the next root. */ }
            }
        }
        throw new FileNotFoundException(url.toString());
    }

    /// A root-scoped URL handler; URLs do not resolve to native filesystem paths.
    private static final class Root extends URLStreamHandler {
        /// Indexed files belonging to this root.
        final ResourceIndex.Root root;
        /// Root code-source URL.
        final URL base;
        /// Decoded root prefix for resource lookup.
        final String prefix;
        /// Sanitized runtime manifest, if present.
        final Manifest manifest;

        /// Creates stable escaped URLs and reads package metadata.
        Root(ResourceIndex.Root root, int id) throws IOException {
            this.root = root;
            prefix = "/" + id + "/" + root.name + "/";
            base = url("");
            ResourceIndex.Resource resource = root.files.get("META-INF/MANIFEST.MF");
            if (resource == null) {
                for (Map.Entry<String, ResourceIndex.Resource> entry : root.files.entrySet()) {
                    if (entry.getKey().equalsIgnoreCase("META-INF/MANIFEST.MF")) {
                        resource = entry.getValue();
                        break;
                    }
                }
            }
            manifest = resource == null ? null : new Manifest(new ByteArrayInputStream(resource.read()));
        }

        /// Returns an escaped URL using this root's handler.
        URL url(String name) {
            try {
                return new URL(null, new URI("janex", null, prefix + name, null).toASCIIString(), this);
            } catch (URISyntaxException | MalformedURLException invalid) {
                throw new IllegalArgumentException(invalid);
            }
        }

        /// Tests manifest sealing with package attributes overriding main attributes.
        boolean sealed(String name) {
            if (manifest == null) return false;
            Attributes attributes = manifest.getAttributes(name.replace('.', '/') + "/");
            String value = attributes == null ? null : attributes.getValue(Attributes.Name.SEALED);
            if (value == null) value = manifest.getMainAttributes().getValue(Attributes.Name.SEALED);
            return "true".equalsIgnoreCase(value);
        }

        /// Opens only an exact resource in this root and never interprets native paths.
        @Override
        protected URLConnection openConnection(URL url) throws IOException {
            final String path;
            try {
                path = url.toURI().getPath();
            } catch (URISyntaxException invalid) {
                throw new IOException(invalid);
            }
            if (url.getAuthority() != null || url.getQuery() != null || url.getRef() != null
                    || path == null || !path.startsWith(prefix)) throw new FileNotFoundException(url.toString());
            final ResourceIndex.Resource resource = root.files.get(path.substring(prefix.length()));
            if (resource == null) throw new FileNotFoundException(url.toString());
            return new URLConnection(url) {
                /// Marks this exact resource connection as established without decoding it.
                @Override
                public void connect() {
                    connected = true;
                }

                /// Returns a new stream over the privately owned logical bytes.
                @Override
                public InputStream getInputStream() throws IOException {
                    connect();
                    return new ByteArrayInputStream(resource.read());
                }

                /// Returns the declared logical length without decoding the resource.
                @Override
                public long getContentLengthLong() {
                    return resource.length;
                }

                /// Returns the declared logical length, which is bounded by Java array limits.
                @Override
                public int getContentLength() {
                    return resource.length;
                }

                /// Grants no additional permission merely because a class has a Janex URL.
                @Override
                public java.security.Permission getPermission() {
                    return null;
                }
            };
        }
    }
}
