# Janex Gradle Plugin

The `org.glavo.janex` plugin packages a Java application and its runtime dependencies directly through
`janex-writer`, without a native CLI. It applies the Java plugin, adds a `janex` extension and a `janexPack` task,
and makes `assemble` depend on the package. Packages are unsigned and include a `java -jar`
launcher by default.

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
The primary input defaults to the project's `jar` output. Runtime dependencies, including project
dependencies, are resolved by Gradle and embedded in order; they are not flattened into the primary
JAR or converted to remote dependency references.

When the application plugin is present, its `mainClass`, `mainModule`, and
`applicationDefaultJvmArgs` supply conventions. Explicit Janex values take precedence.
With `mainModule` set, runtime dependencies default to the module path. Otherwise they default to
the classpath. Both path collections can be replaced using `setFrom(...)`, or extended using
`from(...)`. Without an explicit main class, the writer attempts to infer the entry point from the JAR.

## Packaging options

```kotlin
janex {
    applicationId = "main"
    mainClass = "example.Main"
    // mainModule = "example.app"
    // javaVersion = "vers:jep322/>=17"
    jvmOptions.add("-Xmx512m")
    arguments.addAll("", "two words", "\uD83D\uDE80")
    withLauncher = true
    compression = true
    outputFile = layout.buildDirectory.file("distributions/application.janex")
}
```

`withLauncher = false` omits the appended JAR launcher; the result can still be launched with
`janex run --allow-unsigned`. Arguments are passed as complete strings without shell splitting.
Compression defaults to automatic Zstandard for blobs and table pages, including encoding overhead
in the size comparison. Set `compression = false` to disable it.

The Java writer currently writes blobs without CLASSFILE transforms or publisher
signatures. Packages remain readable by both the Rust Host and Java launcher.

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
disabled for directory dependencies and native prefixes; ordinary JAR-only packages are cacheable.

## Demo

From the repository root:

```shell
./gradlew -p examples/hello janexPack
java -jar examples/hello/build/distributions/janex-hello.janex "User argument"
```

The demo uses `includeBuild`, so it requires no published plugin or repository credentials.

## Publishing

Publish the implementation, Java libraries, sources, Javadoc, and plugin marker to a local Maven repository:

```shell
./gradlew :janex-gradle-plugin:publishAllPublicationsToLocalRepository
```

The repository is generated under `janex-gradle-plugin/build/repository`. All binary outputs stay
in ignored build directories. To consume a published plugin, add that repository under
`pluginManagement.repositories` alongside `mavenCentral()` for the compression dependency, and use
`id("org.glavo.janex") version "0.1.0"`.

For a CNB Maven repository, pass its actual address using `-PjanexPublishUrl=...` and supply publishing
credentials through `ORG_GRADLE_PROJECT_cnbUsername=cnb` and
`ORG_GRADLE_PROJECT_cnbPassword=<publishing-token>` in the CI environment. Then run
`:janex-gradle-plugin:publishAllPublicationsToCnbRepository`. The publishing repository is configured
only when its URL is supplied. Public consumers need only its URL, without credentials.

## Verification

The root `check` includes `:janex-gradle-plugin:check`, which builds the CLI and runs TestKit
fixtures using the Java writer, with the CLI used for interoperability checks. The checks cover classpath and module applications, project dependencies,
resources, preset arguments, both launcher forms, configuration-cache and build-cache reuse, dependency changes,
and preservation of previous output on a failed repack.

For a standalone plugin checkout, use the repository wrapper with
`-p janex-gradle-plugin check -PjanexTestExecutable=/absolute/path/to/janex`. The native launcher
must be built alongside that CLI for the platform's native-prefix checks.
