# Janex Gradle Plugin

The `org.glavo.janex` plugin packages a Java application and its runtime dependencies directly through
`janex-writer`, without a native CLI. It applies the Java plugin, adds a `janex` extension and a `janexPack` task,
and makes `assemble` depend on the package. Packages include a `java -jar` launcher by default;
publisher signing is optional.

The published plugin JAR bundles `janex-reader` and `janex-writer`, including the writer's embedded
bootstrap JAR. Zstd-jni, Bouncy Castle, and ASM remain ordinary Maven dependencies. The source modules
remain separate; using the plugin does not require separate Janex library publications.

The plugin is built with JDK 25, targets Java 17, and is tested with the repository's Gradle 9.7.1
wrapper. Application bytecode can target an older Java version independently.

## Local development

In the consuming project's `settings.gradle.kts`, include the plugin build from your checkout:

```kotlin
pluginManagement {
    includeBuild("../janex/janex-gradle-plugin")
}
```

Configure the application in `build.gradle.kts`:

```kotlin
plugins {
    application
    id("org.glavo.janex")
}

application {
    mainClass = "example.Main"
}

janex {
    arguments.add("--demo")
}
```

The composite build compiles the Java reader, writer, and bootstrap automatically. No native tool
installation or architecture selection is required for ordinary packaging.

Run `janexPack` or `assemble`; the default output is `build/distributions/<project-name>.janex`.
The primary input defaults to `sourceSets.main.output`: compiled classes and processed resources
are merged directly into one ResourceRoot, without creating an intermediate JAR. Producer tasks are
inferred from these inputs. Runtime dependencies, including project dependencies, are resolved by
Gradle and embedded in order as separate roots; they are not converted to remote references.
`janexPack` does not depend on the primary `jar` task. Standard Java `assemble` dependencies remain
unchanged; disable `jar` explicitly if the project should distribute only Janex output.

When the application plugin is present, its `mainClass`, `mainModule`, and
`applicationDefaultJvmArgs` supply conventions. Explicit Janex values take precedence.
With `mainModule` set, runtime dependencies default to the module path. Otherwise they default to
the classpath. Both path collections can be replaced using `setFrom(...)`, or extended using
`from(...)`. Without an explicit main class, the writer attempts to infer the entry point from the
primary manifest or module descriptor.

### Primary inputs and manifest

```kotlin
janex {
    manifestAttributes.putAll(mapOf(
        "Implementation-Version" to project.version.toString(),
        "Multi-Release" to "true",
    ))
    // inputDirectories.from(tasks.named("generateAdditionalResources"))
    // source = tasks.jar.flatMap { it.archiveFile }
}

tasks.processResources {
    from("extra-assets") { into("assets") }
}
```

`inputDirectories` accepts directories and their producer providers; use `setFrom(...)` to replace
the defaults. Missing output directories are skipped. Duplicate directories merge, but duplicate
file or link paths fail rather than silently selecting one input. Primary files use 0644 permissions
and directories 0755; symbolic-link identities and targets participate in the task's cache inputs.
Dependency-directory permissions retain their original behavior.

`sourceName` defaults to `<project-name>.jar` and supplies the merged root's identity, not an output
file. `manifestAttributes` overrides the primary manifest's main attributes and defaults to
`Manifest-Version: 1.0`; existing named sections remain. `Multi-Release` takes effect before versioned
resources are interpreted. Attribute names must be valid and unique ignoring case; values cannot
contain CR, LF, or NUL. All three properties are also available on `JanexPack` tasks.

An explicit `source` selects a JAR and ignores `inputDirectories`, preserving its filename.
Customizations to `tasks.jar` are not automatically imported in direct mode: move resources and path
mappings to `processResources` and manifest attributes to `janex`, or select that JAR explicitly.

## Packaging options

```kotlin
janex {
    applicationId = "main"
    mainClass = "example.Main"
    // mainModule = "example.app"
    javaVersion(17)
    jvmOptions.add("-Xmx512m")
    arguments.addAll("", "two words", "\uD83D\uDE80")
    withLauncher = true
    compression = true
    compressionLevel = 3
    transformClassfiles = true
    outputFile = layout.buildDirectory.file("distributions/application.janex")
}
```

`javaVersion(17)` requires Java 17 or later and sets `vers:jep322/>=17`.
The method rejects integers below 8 immediately. The `Property<String>` accepts full VERS requirements,
for example `javaVersion = "vers:jep322/>=17|<22"`, as well as `.set(...)` and `Provider<String>` values.
Both APIs are available on `JanexPack` tasks. This requirement does not change the Java toolchain or compilation target.

`withLauncher = false` omits the appended JAR launcher; the result can still be launched with
`janex run --allow-unsigned`. Arguments are passed as complete strings without shell splitting.
Compression defaults to automatic Zstandard for blobs and table pages, including encoding overhead
in the size comparison. `compressionLevel` defaults to 3 and also accepts Gradle providers. Zero
selects Zstd's native default; negative levels favor speed. Values outside the loaded library's
supported range fail packaging. Set `compression = false` to disable compression and ignore the level.

Compression uses zstd-jni on the build JVM's platform. Its native library is a build-time dependency;
it is not included in the bootstrap JAR or generated application. Applications continue to use the
portable Java decoder. Each encoding pass reuses a single-threaded context for its blobs and table
pages and closes it when finished. Reproducible builds should pin the plugin and compression-library versions.

CLASSFILE transforms default to enabled. They share constant-pool strings and class-name components
with resource names within each ResourceRoot. Every root uses its own data pool and retains
ordinary class bytes when the complete transformed representation would not shrink.
Set `transformClassfiles = false` to retain ordinary
class bytes. Resource contents remain byte-for-byte reproducible after decoding.

### Dependency minimization and resource filtering

Minimization is disabled by default. Enable it to remove dependency classes that are not reachable
from the application's classes. Explicit keep rules retain reflection or native entry points **and
the classes they reference**, without retaining their entire JAR:

```kotlin
janex {
    minimize {
        keep("example.spi.ReflectiveProvider")
        keep("example.plugins.**")
        // keepJar("dynamic-library-*.jar")
    }
    resources {
        exclude("META-INF/maven/**")
        excludeFrom("jna-*.jar", "com/sun/jna/aix-*/**")
    }
}
```

`minimize()` enables analysis without additional keep rules. All primary-input and module-path
classes, package annotations, and declared service providers are retained as analysis roots.
References from all Multi-Release variants are combined, independently of the build JVM.
Only whole classes are removed; retained bytecode is not rewritten and members are not stripped.
Computed reflection, JNI lookups, and external configuration can require explicit keep rules.
Ordinary resources remain unless explicitly excluded. Analysis requires embedded dependencies;
the Java writer rejects minimization with remote dependency references.

Class patterns use binary names (`example.Outer$Inner` for nested classes): `*` matches within one
name component, `?` matches one character, and `**` crosses components. `example.**` includes
subpackages. Patterns are case-sensitive; unmatched keep rules are permitted.
`keepJar(...)` matches original input filenames and retains all remaining classes in matching inputs.

Resource patterns use `/` separators and apply to logical paths in every Multi-Release layer.
`**/` matches zero or more directories; a trailing `/` excludes a subtree. `excludeFrom(...)`
selects inputs by filename glob; directory inputs use `resources.jar`. Exclusions also work with
minimization disabled. They run first, and a class removed by an exclusion cannot be restored by
`keep(...)`. When minimization finds that a required embedded class was excluded, packaging fails.

The same DSL is available on `JanexPack` tasks. Its `minimization` and `resources` properties support
Gradle providers; `minimization.enabled = false` disables analysis. Selection rules are task inputs
and participate in up-to-date checks, the configuration cache, and the build cache.

### Publisher signing

```kotlin
janex {
    withLauncher = false
    signing {
        cmsCertificate = file("publisher.pem")
        cmsKey = file("publisher-key.pem")
        passwordEnvironment = "JANEX_SIGNING_PASSWORD"
    }
}
```

For OpenPGP, replace `cmsCertificate` and `cmsKey` with `openPgpKey = file("publisher-secret.asc")`.
`openPgpFingerprint` optionally selects a primary key or signing subkey by its full hexadecimal
fingerprint. `algorithm` accepts `org.glavo.janex.writer.SigningAlgorithm` values; absence selects the
key's default. `time` optionally fixes an ISO-8601 signing instant, but does not remove algorithm randomness.
Omit `passwordEnvironment` for unencrypted keys. Passwords are read during task execution.

Signed tasks always execute and bypass the build cache; decoded keys and password values are never
stored in task properties or the configuration cache. Failed signing preserves the previous output.
Signed packages run through an authenticating Janex Host or a native prefix with its embedded public
certificate. `withLauncher` must be false because the standalone JAR launcher has no authentication policy.

### Native launchers

For a native executable, supply a launcher built for the target platform:

```kotlin
janex {
    nativeLauncher = file("launchers/janex-launcher.exe")
    nativeLaunchMode = "bootstrap" // "direct" is also supported
    outputFile = layout.buildDirectory.file("distributions/application.exe")
}
```

The native launcher prefix can target a different
architecture or supported operating system; the plugin does not infer its target from the build
JVM. PE and ELF prefixes are supported. `withLauncher` independently controls whether
the native package also supports `java -jar`.

`JanexPack` can also be registered directly for additional packages; supply its `source` and
`outputFile` properties and the desired entry-point and dependency settings.

The task supports configuration-cache reuse and up-to-date checks. It tracks dependency order,
filenames, file contents, and writer implementation. Shared build caching is supported. Successful repacks
replace the destination; writer failures preserve the previous package. Do not place the output inside
an input directory or use an input file as the destination.
Directory dependencies are always repacked to retain permission changes. Shared build caching is
disabled for directory dependencies, native prefixes, and signing; unsigned JAR-only packages are cacheable.

## Demo

From the repository root:

```shell
./gradlew -p examples/hello janexPack
java -jar examples/hello/build/distributions/janex-hello.janex "User argument"
```

The demo uses `includeBuild`, so it requires no published plugin or repository credentials.

## Publishing

Publish the bundled plugin, its sources and Javadoc, and the plugin marker to a local Maven repository:

```shell
./gradlew :janex-gradle-plugin:publishAllPublicationsToLocalRepository
```

The repository is generated under `janex-gradle-plugin/build/repository`. All binary outputs stay
in ignored build directories. To consume a published plugin, add that repository under
`pluginManagement.repositories` alongside `mavenCentral()` for library dependencies, and use
`id("org.glavo.janex") version "0.1.0-SNAPSHOT"`.

All Java modules and the plugin marker use `workspace.package.version` from the root `Cargo.toml`,
including standalone and composite plugin builds. See [project versioning](../docs/BuildArtifacts.md#versioning)
for the development and release workflow.

The CI workflow publishes snapshots to `https://maven.cnb.cool/Glavo/maven/-/packages/` after all
platform tests pass on `main`. It also supports manual runs on `main`, with the same test requirement.
Publishing uses the repository secret `CNB_PUBLISH_TOKEN`; jobs run serially and skip superseded
commits and versions without the `-SNAPSHOT` suffix. Only `org.glavo.janex:janex-gradle-plugin` and
the marker `org.glavo.janex:org.glavo.janex.gradle.plugin` are published. The marker is a POM that
maps the plugin ID to its implementation, not another plugin JAR.

Packaging uses Shadow with the official Gradle Plugin Publish plugin, so Maven publishing and a future
Plugin Portal release use the same bundled implementation. CI only invokes CNB Maven publishing.

To consume a snapshot, configure `settings.gradle.kts`:

```kotlin
pluginManagement {
    repositories {
        maven { url = uri("https://maven.cnb.cool/Glavo/maven/-/packages/") }
        gradlePluginPortal()
        mavenCentral()
    }
}
```

Then use `id("org.glavo.janex") version "0.1.0-SNAPSHOT"` in `build.gradle.kts`.
Public consumers do not need publishing credentials.

For local publishing, pass `-PjanexPublishUrl=https://maven.cnb.cool/Glavo/maven/-/packages/` to
`:janex-gradle-plugin:publishAllPublicationsToCnbRepository`, with credentials supplied through
`ORG_GRADLE_PROJECT_cnbUsername=cnb` and `ORG_GRADLE_PROJECT_cnbPassword=<publishing-token>`.

## Verification

The root `check` includes `:janex-gradle-plugin:check`, which builds the CLI and runs TestKit
fixtures using the Java writer, with the CLI used for interoperability checks. The checks cover classpath and module applications, project dependencies,
resources, preset arguments, both launcher forms, configuration-cache and build-cache reuse, dependency changes,
and preservation of previous output on a failed repack. TestKit loads the bundled plugin JAR;
publication tests resolve both Gradle metadata and POM-only metadata with internal Janex modules
excluded from all repositories.

For a standalone plugin checkout, use the repository wrapper with
`-p janex-gradle-plugin check -PjanexTestExecutable=/absolute/path/to/janex`. The native launcher
must be built alongside that CLI for the platform's native-prefix checks.
