// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap.loader;

import java.io.*;
import java.lang.module.*;
import java.lang.reflect.*;
import java.net.URI;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.jar.Attributes;
import java.util.stream.Stream;

import org.glavo.janex.bootstrap.Bootstrap;

/// Integrates indexed modules with the system resource loader on Java 9 or later.
///
/// Application modules occupy a child of the native boot layer. System modules remain in the
/// boot layer. The launcher enables the narrowly scoped JDK module-access bridge used to apply
/// deferred access options, including access to future unnamed modules.
public final class ModuleSupport {
    /// Prevents instantiation.
    private ModuleSupport() {
    }

    /// Creates a layer over indexed roots and applies launch access options without invoking application code.
    ///
    /// @param loader system resource loader under construction
    /// @throws Exception if descriptors, dependencies, module requirements, or access options are invalid
    public static void initialize(ResourceLoader loader) throws Exception {
        List<String> options = new ArrayList<String>();
        String mainModule;
        String mainClass;
        try (DataInputStream input = new DataInputStream(org.glavo.janex.bootstrap.LaunchData.open(1))) {
            mainModule = text(input);
            mainClass = text(input);
            int count = input.readInt();
            if (count < 0) {
                throw new IOException("Invalid JVM option count");
            }
            for (int i = 0; i < count; i++) {
                options.add(text(input));
            }
            if (input.read() != -1) {
                throw new IOException("Trailing JVM option data");
            }
        }
        if (loader.moduleRoots.isEmpty() && mainModule.isEmpty() && !hasModuleOptions(options)
                && loader.index.requirements.isEmpty()) {
            loader.moduleLayer = ModuleLayer.boot();
            return;
        }
        Finder finder = new Finder(loader);
        ModuleFinder system = ModuleFinder.ofSystem();
        for (Map.Entry<String, String> requirement : loader.index.requirements.entrySet()) {
            ModuleDescriptor descriptor = finder.find(requirement.getKey())
                    .or(() -> system.find(requirement.getKey()))
                    .orElseThrow(() -> new FindException("Required module is unavailable: " + requirement.getKey()))
                    .descriptor();
            if (!requirement.getValue().isEmpty() && !descriptor.rawVersion().orElse("").equals(requirement.getValue())) {
                throw new FindException("required module version is unavailable: " + requirement.getKey() + "@" + requirement.getValue());
            }
        }
        Set<String> roots = new LinkedHashSet<String>(finder.references.keySet());
        if (!mainModule.isEmpty()) {
            roots.add(mainModule);
        }
        for (int i = 0; i < options.size(); i++) {
            String option = options.get(i);
            if (option.equals("--add-modules")) {
                option += "=" + argument(options, ++i);
            }
            if (option.startsWith("--add-modules=")) {
                for (String name : option.substring(14).split(",", -1)) {
                    if (!name.equals("ALL-MODULE-PATH") && !name.equals("ALL-SYSTEM") && !name.equals("ALL-DEFAULT")) {
                        roots.add(name);
                    }
                }
            }
        }
        Configuration configuration = ModuleLayer.boot().configuration().resolveAndBind(finder, ModuleFinder.of(), roots);
        ModuleLayer.Controller controller = ModuleLayer.defineModules(configuration, Collections.singletonList(ModuleLayer.boot()), name -> loader);
        ModuleLayer layer = controller.layer();
        loader.moduleLayer = layer;
        for (Map.Entry<String, ResourceLoader.Root> entry : loader.moduleRoots.entrySet()) {
            Module module = layer.findModule(entry.getKey()).orElseThrow(() -> new FindException(entry.getKey()));
            entry.getValue().module = module;
        }
        applyOptions(layer, options);
        if (!mainModule.isEmpty()) {
            Module module = module(layer, mainModule);
            String entry = mainClass.isEmpty() ? module.getDescriptor().mainClass().orElseThrow(() -> new FindException("Module has no main class: " + mainModule)) : mainClass;
            int dot = entry.lastIndexOf('.');
            if (dot > 0) {
                access("addOpens", module, entry.substring(0, dot), Bootstrap.class.getModule());
            }
        }
    }

    /// Returns whether launch options request module roots or deferred module access changes.
    private static boolean hasModuleOptions(List<String> options) {
        for (String option : options) {
            int equal = option.indexOf('=');
            String key = equal < 0 ? option : option.substring(0, equal);
            switch (key) {
                case "--add-modules":
                case "--add-reads":
                case "--add-exports":
                case "--add-opens":
                case "--enable-native-access":
                    return true;
                default:
                    break;
            }
        }
        return false;
    }

    /// Returns a required option operand.
    private static String argument(List<String> options, int index) {
        if (index >= options.size()) {
            throw new IllegalArgumentException("Missing module option operand");
        }
        return options.get(index);
    }

    /// Looks up an application or parent-layer module by name.
    private static Module module(ModuleLayer layer, String name) {
        return layer.findModule(name).orElseThrow(() -> new FindException("Unknown module: " + name));
    }

    /// Applies the same access changes to child and system modules, retaining ALL-UNNAMED semantics.
    private static void applyOptions(ModuleLayer layer, List<String> options) throws Exception {
        Set<String> accessOptions = new HashSet<String>(Arrays.asList("--add-reads", "--add-exports", "--add-opens", "--enable-native-access"));
        for (int i = 0; i < options.size(); i++) {
            String option = options.get(i);
            int equal = option.indexOf('=');
            String key = equal < 0 ? option : option.substring(0, equal);
            if (!accessOptions.contains(key)) {
                continue;
            }
            String value = equal < 0 ? argument(options, ++i) : option.substring(equal + 1);
            if (key.equals("--enable-native-access")) {
                for (String name : value.split(",", -1)) {
                    if (name.equals("ALL-UNNAMED")) {
                        continue;
                    }
                    Method enable = Module.class.getDeclaredMethod("implAddEnableNativeAccess");
                    enable.setAccessible(true);
                    enable.invoke(module(layer, name));
                }
                continue;
            }
            int split = value.indexOf('=');
            if (split <= 0 || split == value.length() - 1) {
                throw new IllegalArgumentException("Invalid module option: " + option);
            }
            String source = value.substring(0, split);
            String operation = key.equals("--add-reads") ? "addReads" : key.equals("--add-exports") ? "addExports" : "addOpens";
            int slash = source.indexOf('/');
            if (!operation.equals("addReads") && (slash <= 0 || slash == source.length() - 1)) {
                throw new IllegalArgumentException("Missing module package: " + source);
            }
            Module from = module(layer, slash < 0 ? source : source.substring(0, slash));
            String pkg = slash < 0 ? null : source.substring(slash + 1);
            if (pkg != null && !from.getPackages().contains(pkg)) {
                throw new IllegalArgumentException("Unknown module package: " + source);
            }
            for (String target : value.substring(split + 1).split(",", -1)) {
                if (target.equals("ALL-UNNAMED")) {
                    access(operation + "AllUnnamed", from, pkg, null);
                } else {
                    access(operation, from, pkg, module(layer, target));
                }
            }
        }
    }

    /// Calls the JDK module-access bridge explicitly enabled by the native launch arguments.
    private static void access(String operation, Module from, String pkg, Module target) throws Exception {
        boolean all = operation.endsWith("AllUnnamed");
        String method = all && !operation.startsWith("addReads") ? operation.replace("AllUnnamed", "ToAllUnnamed") : operation;
        Class<?> bridge = Class.forName("jdk.internal.module.Modules");
        Class<?>[] types = pkg == null ? (all ? new Class<?>[]{Module.class} : new Class<?>[]{Module.class, Module.class})
                : (all ? new Class<?>[]{Module.class, String.class} : new Class<?>[]{Module.class, String.class, Module.class});
        Object[] arguments = pkg == null ? (all ? new Object[]{from} : new Object[]{from, target})
                : (all ? new Object[]{from, pkg} : new Object[]{from, pkg, target});
        bridge.getMethod(method, types).invoke(null, arguments);
    }

    /// Loads the selected main class from its actual module without initializing it.
    ///
    /// @param loader     active system resource loader
    /// @param moduleName selected named module
    /// @param className  explicit binary name, or empty to use the module descriptor
    /// @return the entry class belonging to the requested module
    /// @throws ClassNotFoundException if the selected module does not contain the entry class
    public static Class<?> mainClass(ResourceLoader loader, String moduleName, String className) throws ClassNotFoundException {
        Module module = module((ModuleLayer) loader.moduleLayer, moduleName);
        String name = className.isEmpty() ? module.getDescriptor().mainClass().orElseThrow(() -> new FindException("Module has no main class: " + moduleName)) : className;
        Class<?> result = Class.forName(module, name);
        if (result == null) {
            throw new ClassNotFoundException(name);
        }
        return result;
    }

    /// Computes native system-module roots from descriptor metadata without resolving a module graph.
    ///
    /// @param data selected module resources, or null when there are no module entries or requirements
    /// @param mainModule main module name, or an empty string for classpath launching
    /// @param options ordered application JVM options
    /// @return system-module names and native root tokens to enable in the application JVM
    /// @throws IOException if indexed metadata cannot be read
    /// @throws FindException if a required module or exact version is unavailable
    public static String[] launchModules(org.glavo.janex.reader.ResourcePlan data, String mainModule, List<String> options) throws IOException {
        ModuleFinder system = ModuleFinder.ofSystem();
        Map<String, ModuleDescriptor> available = new HashMap<String, ModuleDescriptor>();
        Set<String> roots = new TreeSet<String>();
        for (ModuleReference reference : system.findAll()) {
            available.put(reference.descriptor().name(), reference.descriptor());
        }
        List<ModuleDescriptor> selected = new ArrayList<ModuleDescriptor>();
        if (data != null) {
            try (ResourceIndex index = new ResourceIndex(data)) {
                for (ResourceIndex.Root root : index.roots) {
                    if (!root.module) {
                        continue;
                    }
                    ResourceIndex.Resource info = root.files.get("module-info.class");
                    ModuleDescriptor descriptor;
                    if (info != null) {
                        descriptor = ModuleDescriptor.read(ByteBuffer.wrap(info.read()));
                    } else {
                        java.util.jar.Manifest manifest = null;
                        for (Map.Entry<String, ResourceIndex.Resource> entry : root.files.entrySet()) {
                            if (entry.getKey().equalsIgnoreCase("META-INF/MANIFEST.MF")) {
                                manifest = new java.util.jar.Manifest(new ByteArrayInputStream(entry.getValue().read()));
                                break;
                            }
                        }
                        descriptor = automaticIdentity(root.name, manifest).build();
                    }
                    if (available.putIfAbsent(descriptor.name(), descriptor) != null) {
                        throw new FindException("Duplicate or shadowed module: " + descriptor.name());
                    }
                    selected.add(descriptor);
                }
                for (Map.Entry<String, String> requirement : index.requirements.entrySet()) {
                    ModuleDescriptor descriptor = available.get(requirement.getKey());
                    if (descriptor == null) {
                        throw new FindException("Required module is unavailable: " + requirement.getKey());
                    }
                    if (!requirement.getValue().isEmpty() && !descriptor.rawVersion().orElse("").equals(requirement.getValue())) {
                        throw new FindException("Required module version is unavailable: " + requirement.getKey() + "@" + requirement.getValue());
                    }
                }
            }
        }
        Set<String> required = new LinkedHashSet<String>();
        if (!mainModule.isEmpty()) {
            required.add(mainModule);
        }
        for (ModuleDescriptor descriptor : selected) {
            for (ModuleDescriptor.Requires dependency : descriptor.requires()) {
                if (!dependency.modifiers().contains(ModuleDescriptor.Requires.Modifier.STATIC)) {
                    required.add(dependency.name());
                }
            }
        }
        for (String name : required) {
            if (!available.containsKey(name)) {
                throw new FindException("Required module is unavailable: " + name);
            }
            if (system.find(name).isPresent()) {
                roots.add(name);
            }
        }
        for (int i = 0; i < options.size(); i++) {
            String option = options.get(i);
            if (option.equals("--add-modules")) {
                option += "=" + argument(options, ++i);
            }
            if (option.startsWith("--add-modules=")) {
                for (String name : option.substring(14).split(",", -1)) {
                    if (name.equals("ALL-SYSTEM") || name.equals("ALL-DEFAULT") || system.find(name).isPresent()) {
                        roots.add(name);
                    }
                }
            }
        }
        return roots.toArray(new String[0]);
    }

    /// Derives an automatic module's name and optional version without reading its packages or services.
    private static ModuleDescriptor.Builder automaticIdentity(String file, java.util.jar.Manifest manifest) {
        String stem = file.endsWith(".jar") ? file.substring(0, file.length() - 4) : file;
        java.util.regex.Matcher version = java.util.regex.Pattern.compile("-(\\d+(?:\\.|$))").matcher(stem);
        String rawVersion = null;
        if (version.find()) {
            rawVersion = stem.substring(version.start() + 1);
            stem = stem.substring(0, version.start());
        }
        String name = manifest == null ? null : manifest.getMainAttributes().getValue("Automatic-Module-Name");
        if (name == null) {
            name = stem.replaceAll("[^A-Za-z0-9]", ".").replaceAll("\\.+", ".").replaceAll("^\\.|\\.$", "");
        }
        ModuleDescriptor.Builder builder = ModuleDescriptor.newAutomaticModule(name);
        if (rawVersion != null) {
            try {
                builder.version(rawVersion);
            } catch (IllegalArgumentException unparseable) { /* Optional filename version. */ }
        }
        return builder;
    }

    /// Reads a nonnegative counted sequence of UTF-16 code units from private launch data.
    private static String text(DataInputStream input) throws IOException {
        int count = input.readInt();
        if (count < 0) {
            throw new IOException("Invalid option string length");
        }
        char[] value = new char[count];
        for (int i = 0; i < count; i++) {
            value[i] = input.readChar();
        }
        return new String(value);
    }

    /// Finds descriptors over indexed resources without extracting module JARs.
    private static final class Finder implements ModuleFinder {
        /// Immutable references indexed by their validated descriptor names.
        final Map<String, ModuleReference> references = new LinkedHashMap<String, ModuleReference>();

        /// Reads descriptors, derives automatic-module metadata, and records package ownership.
        Finder(ResourceLoader loader) throws IOException {
            for (ResourceLoader.Root root : loader.allRoots) {
                if (!root.root.module) {
                    continue;
                }
                Set<String> packages = packages(root);
                ResourceIndex.Resource info = root.root.files.get("module-info.class");
                ModuleDescriptor descriptor = info == null ? automatic(root, packages) : ModuleDescriptor.read(ByteBuffer.wrap(info.read()), () -> packages);
                String name = descriptor.name();
                if (ModuleLayer.boot().findModule(name).isPresent()) {
                    throw new FindException("Module shadows a system module: " + name);
                }
                if (references.put(name, new Reference(root, descriptor)) != null) {
                    throw new FindException("Duplicate module: " + name);
                }
                loader.moduleRoots.put(name, root);
                for (String pkg : descriptor.packages()) {
                    if (loader.modulePackages.put(pkg, root) != null) {
                        throw new LayerInstantiationException("Split module package: " + pkg);
                    }
                }
            }
        }

        /// Collects module packages and rejects classes in the unnamed package.
        private static Set<String> packages(ResourceLoader.Root root) {
            Set<String> packages = new HashSet<String>();
            for (String path : root.root.files.keySet()) {
                if (!path.endsWith(".class") || path.equals("module-info.class") || path.startsWith("META-INF/")) {
                    continue;
                }
                int slash = path.lastIndexOf('/');
                if (slash < 0) {
                    throw new InvalidModuleDescriptorException("Unnamed package in module: " + path);
                }
                packages.add(path.substring(0, slash).replace('/', '.'));
            }
            return packages;
        }

        /// Derives an automatic module from its original JAR filename, manifest, packages, and service files.
        private static ModuleDescriptor automatic(ResourceLoader.Root root, Set<String> packages) throws IOException {
            ModuleDescriptor.Builder builder = automaticIdentity(root.root.name, root.manifest).packages(packages);
            for (Map.Entry<String, ResourceIndex.Resource> entry : root.root.files.entrySet()) {
                String prefix = "META-INF/services/";
                if (!entry.getKey().startsWith(prefix) || entry.getValue().id == -1) {
                    continue;
                }
                String service = entry.getKey().substring(prefix.length());
                if (service.isEmpty() || service.contains("/")) {
                    continue;
                }
                List<String> providers = new ArrayList<String>();
                try (BufferedReader input = new BufferedReader(new InputStreamReader(new ByteArrayInputStream(entry.getValue().read()), StandardCharsets.UTF_8.newDecoder()))) {
                    String line;
                    while ((line = input.readLine()) != null) {
                        int hash = line.indexOf('#');
                        if (hash >= 0) {
                            line = line.substring(0, hash);
                        }
                        line = line.trim();
                        if (line.isEmpty()) {
                            continue;
                        }
                        int dot = line.lastIndexOf('.');
                        if (dot < 0 || !packages.contains(line.substring(0, dot))) {
                            throw new InvalidModuleDescriptorException("Provider is not in module: " + line);
                        }
                        if (!providers.contains(line)) {
                            providers.add(line);
                        }
                    }
                }
                if (!providers.isEmpty()) {
                    builder.provides(service, providers);
                }
            }
            String main = root.manifest == null ? null : root.manifest.getMainAttributes().getValue(Attributes.Name.MAIN_CLASS);
            if (main != null) {
                main = main.replace('/', '.');
                int dot = main.lastIndexOf('.');
                if (dot > 0 && packages.contains(main.substring(0, dot))) {
                    builder.mainClass(main);
                }
            }
            return builder.build();
        }

        /// Returns the named indexed module, if present.
        @Override
        public Optional<ModuleReference> find(String name) {
            return Optional.ofNullable(references.get(Objects.requireNonNull(name)));
        }

        /// Returns a stable immutable set of all indexed module references.
        @Override
        public Set<ModuleReference> findAll() {
            return Collections.unmodifiableSet(new LinkedHashSet<ModuleReference>(references.values()));
        }
    }

    /// A module descriptor bound to its resource root URI and independent readers.
    private static final class Reference extends ModuleReference {
        /// Resource root shared with URL and NIO access.
        private final ResourceLoader.Root root;

        /// Creates a reference without opening resources beyond the descriptor.
        Reference(ResourceLoader.Root root, ModuleDescriptor descriptor) {
            super(descriptor, URI.create(root.base.toString()));
            this.root = root;
        }

        /// Returns an independently closeable module reader.
        @Override
        public ModuleReader open() {
            return new Reader(root);
        }
    }

    /// Reads exact module resources; returned buffers are read-only and streams own no snapshot handle.
    private static final class Reader implements ModuleReader {
        /// Root whose resources this reader exposes.
        private final ResourceLoader.Root root;
        /// Reader-local closed state; it does not close the loader.
        private final AtomicBoolean closed = new AtomicBoolean();

        /// Creates an open reader over an immutable resource root.
        Reader(ResourceLoader.Root root) {
            this.root = root;
        }

        /// Rejects operations after reader closure.
        private void check() throws IOException {
            if (closed.get() || root.root.isClosed()) {
                throw new IOException("Module reader is closed");
            }
        }

        /// Rejects an empty name rather than exposing the module root as an unnamed resource.
        private ResourceIndex.Resource resource(String name) {
            return Objects.requireNonNull(name).isEmpty() ? null : root.resource(name);
        }

        /// Returns the URI of an existing exact resource.
        @Override
        public Optional<URI> find(String name) throws IOException {
            check();
            return resource(name) != null ? Optional.of(URI.create(root.url(name).toString())) : Optional.empty();
        }

        /// Returns a new read-only buffer over immutable logical bytes.
        @Override
        public Optional<ByteBuffer> read(String name) throws IOException {
            check();
            ResourceIndex.Resource resource = resource(name);
            return resource == null ? Optional.empty() : Optional.of(ByteBuffer.wrap(resource.read()).asReadOnlyBuffer());
        }

        /// Returns an independent input stream for a file or directory resource.
        @Override
        public Optional<InputStream> open(String name) throws IOException {
            check();
            ResourceIndex.Resource resource = resource(name);
            return resource == null ? Optional.empty() : Optional.of(new ByteArrayInputStream(resource.read()));
        }

        /// Releases no native memory; returned buffers retain their contents after reader closure.
        @Override
        public void release(ByteBuffer buffer) {
            Objects.requireNonNull(buffer);
        }

        /// Lists all resource names in index order without decoding payloads.
        @Override
        public Stream<String> list() throws IOException {
            check();
            return root.root.files.keySet().stream().filter(name -> !name.isEmpty());
        }

        /// Closes this reader without invalidating already returned buffers or streams.
        @Override
        public void close() {
            closed.set(true);
        }
    }
}
