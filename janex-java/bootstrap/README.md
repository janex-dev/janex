# Java Bootstrap

`Bootstrap.class` is compiled for Java 8 and embedded in `janex-java`. Building Janex does not
require a JDK. Rebuild it after editing the source with JDK 25:

```text
python janex-java/bootstrap/build.py
python janex-java/bootstrap/build.py --check
```

Each bootstrap launch creates a private JAR containing this class and `launch.bin`. The latter
stores the module name, class name, a boolean selecting modern main-method rules, and program
arguments. Strings contain a big-endian nonnegative 32-bit UTF-16 code-unit count followed by
those code units; the argument array also has a 32-bit count. This is private launch data, not
part of the Janex file format. It preserves empty strings, NUL, supplementary characters, and Windows
surrogate code units without placing program arguments on the native Java command line.

Classpath launches prepend the bootstrap JAR. Module launches patch it into the selected main
module, keeping that module as the root and allowing access to its main class without opening
application packages to unrelated modules. The original module descriptor still supplies the
main class when none is explicitly selected. Application agents run before bootstrap entry.

The bootstrap invokes the main method on the main thread and lets exceptions and process exit
propagate. Native JVM options, paths, and agent options still pass through the Java launcher.
Direct mode bypasses the bootstrap entirely; it remains available for runtimes or application
entry mechanisms that cannot use this layer.
