// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.*;
import java.net.*;
import java.security.CodeSource;
import java.security.cert.Certificate;
import java.util.*;
import java.util.jar.*;

/// Loads Host-selected classpath and module roots through standard parent-first delegation.
///
/// The JVM installs this as its system loader. Resource lookup retains root order and
/// duplicates, package manifest attributes, sealing, and one code-source URL per root.
public final class ResourceLoader extends URLClassLoader {
    static {
        registerAsParallelCapable();
    }

    /// Owned resource snapshot and decoded blob cache.
    final ResourceIndex index;
    /// Active system loader; installed after initialization has completed.
    private static volatile ResourceLoader active;
    /// Lazily mounted NIO view, whose lifecycle is independent of class loading.
    private JanexFileSystem fileSystem;
    /// Every indexed root in URI order, including module roots.
    final List<Root> allRoots = new ArrayList<Root>();
    /// Named modules assigned to this loader by the Java 9+ bridge.
    final Map<String, Root> moduleRoots = new LinkedHashMap<String, Root>();
    /// Package ownership for named-module class loading.
    final Map<String, Root> modulePackages = new HashMap<String, Root>();
    /// Defined module layer, accessed by the Java 9+ bridge.
    Object moduleLayer;
    /// System-module roots needed by the indexed application graph.
    final Set<String> systemModules = new TreeSet<String>();
    /// URL handlers and manifests in classpath order.
    private final List<Root> roots = new ArrayList<Root>();
    /// Packages defined by this loader, excluding packages belonging to parents.
    private final Map<String, Package> packages = new HashMap<String, Package>();
    /// Marks a local definition so Java 8 does not confuse a parent's package with this loader's package.
    private final ThreadLocal<Boolean> definingPackage = new ThreadLocal<Boolean>();

    /// Creates the JVM system loader using the private index in its own bootstrap JAR.
    ///
    /// @param parent the JVM's original application loader
    /// @throws IOException if the index or a runtime manifest cannot be read
    public ResourceLoader(ClassLoader parent) throws IOException {
        super(new URL[0], parent);
        InputStream data = ResourceLoader.class.getResourceAsStream("resources.bin");
        if (data == null) {
            throw new IOException("Missing Janex resource index");
        }
        index = new ResourceIndex(data);
        try {
            for (ResourceIndex.Root root : index.roots) {
                Root entry = new Root(root, allRoots.size());
                allRoots.add(entry);
                if (!root.module) {
                    roots.add(entry);
                }
            }
            try {
                Class.forName("org.janex.bootstrap.ModuleSupport").getMethod("initialize", ResourceLoader.class).invoke(null, this);
            } catch (ClassNotFoundException java8) {
                if (!index.requirements.isEmpty()) {
                    throw new IOException("Module requirements need Java 9 or later");
                }
                for (Root root : allRoots) {
                    if (root.root.module) {
                        throw new IOException("Modules require Java 9 or later");
                    }
                }
            } catch (ReflectiveOperationException failure) {
                throw new IOException("Cannot initialize Janex modules", failure.getCause() == null ? failure : failure.getCause());
            }
            String handlers = System.getProperty("java.protocol.handler.pkgs", "");
            String prefix = "org.janex.bootstrap.protocol";
            if (!Arrays.asList(handlers.split("\\|", -1)).contains(prefix)) {
                System.setProperty("java.protocol.handler.pkgs", handlers.isEmpty() ? prefix : handlers + "|" + prefix);
            }
            active = this;
        } catch (Throwable failure) {
            try {
                index.close();
            } catch (IOException close) {
                failure.addSuppressed(close);
            }
            throw failure;
        }
    }

    /// Preserves inherited lookup while allowing independent package definitions on Java 8.
    @Override
    protected Package getPackage(String name) {
        synchronized (packages) {
            if (Boolean.TRUE.equals(definingPackage.get())) {
                return packages.get(name);
            }
            return super.getPackage(name);
        }
    }

    /// Records local packages, including packages defined from instrumentation-appended JARs.
    @Override
    protected Package definePackage(String name, String specTitle, String specVersion, String specVendor,
                                    String implTitle, String implVersion, String implVendor, URL sealBase) {
        synchronized (packages) {
            definingPackage.set(true);
            try {
                Package result = super.definePackage(name, specTitle, specVersion, specVendor, implTitle, implVersion, implVendor, sealBase);
                packages.put(name, result);
                return result;
            } finally {
                definingPackage.remove();
            }
        }
    }

    /// Defines the first matching class after inherited parent delegation has failed.
    @Override
    protected Class<?> findClass(String name) throws ClassNotFoundException {
        int separator = name.lastIndexOf('.');
        Root module = modulePackages.get(separator < 0 ? "" : name.substring(0, separator));
        if (module != null) {
            return define(name, module);
        }
        String path = name.replace('.', '/') + ".class";
        for (Root root : roots) {
            ResourceIndex.Resource resource = root.root.files.get(path);
            if (resource == null || resource.id == -1) {
                continue;
            }
            try {
                byte[] bytes = resource.read();
                int dot = name.lastIndexOf('.');
                if (dot != -1) {
                    ensurePackage(name.substring(0, dot), root);
                }
                return defineClass(name, bytes, 0, bytes.length, new CodeSource(root.base, (Certificate[]) null));
            } catch (IOException failure) {
                throw new ClassNotFoundException(name, failure);
            }
        }
        return super.findClass(name);
    }

    /// Defines a class from one exact module root without searching unrelated roots.
    private Class<?> define(String name, Root root) throws ClassNotFoundException {
        ResourceIndex.Resource resource = root.root.files.get(name.replace('.', '/') + ".class");
        if (resource == null || resource.id == -1) {
            throw new ClassNotFoundException(name);
        }
        try {
            byte[] bytes = resource.read();
            return defineClass(name, bytes, 0, bytes.length, new CodeSource(root.base, (Certificate[]) null));
        } catch (IOException failure) {
            throw new ClassNotFoundException(name, failure);
        }
    }

    /// Implements the Java 9 module-aware class-loading hook while retaining Java 8 bytecode.
    protected Class<?> findClass(String moduleName, String name) {
        Root root = moduleRoots.get(moduleName);
        if (root == null) {
            return null;
        }
        int dot = name.lastIndexOf('.');
        if (modulePackages.get(dot < 0 ? "" : name.substring(0, dot)) != root) {
            return null;
        }
        synchronized (getClassLoadingLock(name)) {
            Class<?> loaded = findLoadedClass(name);
            if (loaded != null) {
                return loaded;
            }
            try {
                return define(name, root);
            } catch (ClassNotFoundException absent) {
                return null;
            }
        }
    }

    /// Implements exact module resource lookup; the JVM applies module access checks.
    protected URL findResource(String moduleName, String name) {
        Root root = moduleRoots.get(moduleName);
        return root != null && root.resource(name) != null ? root.url(name) : null;
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

    /// Returns the first visible module or classpath resource, followed by agent-appended JARs.
    @Override
    public URL findResource(String name) {
        for (Root root : moduleRoots.values()) {
            if (visible(root, name)) {
                return root.url(name);
            }
        }
        for (Root root : roots) {
            if (root.resource(name) != null) {
                return root.url(name);
            }
        }
        return super.findResource(name);
    }

    /// Enumerates all local matches without collapsing equal resource names across roots.
    @Override
    public Enumeration<URL> findResources(String name) throws IOException {
        List<URL> matches = new ArrayList<URL>();
        for (Root root : moduleRoots.values()) {
            if (visible(root, name)) {
                matches.add(root.url(name));
            }
        }
        for (Root root : roots) {
            if (root.resource(name) != null) {
                matches.add(root.url(name));
            }
        }
        Enumeration<URL> appended = super.findResources(name);
        while (appended.hasMoreElements()) {
            matches.add(appended.nextElement());
        }
        return Collections.enumeration(matches);
    }

    /// Tests module resource visibility for ordinary class-loader lookup.
    private boolean visible(Root root, String name) {
        if (root.resource(name) == null) {
            return false;
        }
        if (name.endsWith(".class") || !name.contains("/")) {
            return true;
        }
        String pkg = name.substring(0, name.lastIndexOf('/')).replace('/', '.');
        if (!modulePackages.containsKey(pkg)) {
            return true;
        }
        try {
            return (Boolean) Class.forName("java.lang.Module").getMethod("isOpen", String.class).invoke(root.module, pkg);
        } catch (ReflectiveOperationException failure) {
            throw new IllegalStateException(failure);
        }
    }

    /// Obtains or remounts the system loader's closeable NIO view.
    static JanexFileSystem fileSystem(JanexFileSystemProvider provider, boolean create) {
        ResourceLoader loader = active;
        if (loader == null || loader.index.isClosed()) {
            throw new java.nio.file.FileSystemNotFoundException("No active Janex snapshot");
        }
        synchronized (loader) {
            if (loader.fileSystem == null) {
                loader.fileSystem = new JanexFileSystem(provider, loader.index);
            } else if (create) {
                if (loader.fileSystem.isOpen()) {
                    throw new java.nio.file.FileSystemAlreadyExistsException();
                }
                loader.fileSystem = new JanexFileSystem(provider, loader.index);
            } else if (!loader.fileSystem.isOpen()) {
                throw new java.nio.file.FileSystemNotFoundException("Janex view is closed");
            }
            return loader.fileSystem;
        }
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
            for (Root root : ((ResourceLoader) loader).allRoots) {
                try {
                    return root.openConnection(url);
                } catch (FileNotFoundException absent) { /* Continue with the next root. */ }
            }
        }
        throw new FileNotFoundException(url.toString());
    }

    /// A root-scoped URL handler; URLs do not resolve to native filesystem paths.
    static final class Root extends URLStreamHandler {
        /// Indexed files belonging to this root.
        final ResourceIndex.Root root;
        /// Root code-source URL.
        final URL base;
        /// Decoded root prefix for resource lookup.
        final String prefix;
        /// Sanitized runtime manifest, if present.
        final Manifest manifest;
        /// Java 9 module identity, or null for classpath roots and before layer definition.
        Object module;

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
                ResourceIndex.Resource resource = resource(name);
                if (!name.isEmpty() && !name.endsWith("/") && resource != null && resource.id == -1) {
                    name += "/";
                }
                return new URL(null, new URI("janex", null, prefix + name, null).toASCIIString(), this);
            } catch (URISyntaxException | MalformedURLException invalid) {
                throw new IllegalArgumentException(invalid);
            }
        }

        /// Resolves an exact file name or a directory with either trailing-separator form.
        ResourceIndex.Resource resource(String name) {
            ResourceIndex.Resource result = root.files.get(name);
            if (result == null && name.endsWith("/")) {
                result = root.files.get(name.substring(0, name.length() - 1));
                if (result != null && result.id != -1) {
                    return null;
                }
            }
            if (result == null && !name.endsWith("/")) {
                result = root.files.get(name + "/");
                if (result != null && result.id != -1) {
                    return null;
                }
            }
            return result;
        }

        /// Tests manifest sealing with package attributes overriding main attributes.
        boolean sealed(String name) {
            if (manifest == null) {
                return false;
            }
            Attributes attributes = manifest.getAttributes(name.replace('.', '/') + "/");
            String value = attributes == null ? null : attributes.getValue(Attributes.Name.SEALED);
            if (value == null) {
                value = manifest.getMainAttributes().getValue(Attributes.Name.SEALED);
            }
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
                    || path == null || !(path.startsWith(prefix) || path.equals(prefix.substring(0, prefix.length() - 1)))) {
                throw new FileNotFoundException(url.toString());
            }
            final ResourceIndex.Resource resource = resource(path.length() < prefix.length() ? "" : path.substring(prefix.length()));
            if (resource == null) {
                throw new FileNotFoundException(url.toString());
            }
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
