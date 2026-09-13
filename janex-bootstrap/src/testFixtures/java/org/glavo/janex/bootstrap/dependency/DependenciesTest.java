// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap.dependency;

import java.io.IOException;

/// Checks external address mapping independently of transport and cache availability.
public final class DependenciesTest {
    /// Verifies directory precedence and invalid overrides in isolated JVM environments.
    private static void cacheDirectories() throws Exception {
        java.nio.file.Path root = java.nio.file.Paths.get(System.getProperty("java.io.tmpdir"))
                .toAbsolutePath().resolve("Janex Home");
        String homeVariable = System.getProperty("os.name").startsWith("Windows") ? "USERPROFILE" : "HOME";
        String[] overrides = {null, root.toString(), "", "relative/path", "~/.janex", "relative", null};
        for (int index = 0; index < overrides.length; index++) {
            String expected = index == 0 ? root.resolve(".janex/cache/dependencies").toString()
                    : index == 1 ? root.resolve("cache/dependencies").toString()
                    : index == 5 ? root.resolve("explicit").toString() : "error";
            java.util.List<String> command = new java.util.ArrayList<String>();
            command.add(java.nio.file.Paths.get(System.getProperty("java.home"), "bin", "java").toString());
            if (index == 5) {
                command.add("-Djanex.dependencyCache=" + expected);
            }
            java.util.Collections.addAll(command, "-cp", System.getProperty("java.class.path"),
                    DependenciesTest.class.getName(), expected);
            ProcessBuilder builder = new ProcessBuilder(command).inheritIO();
            builder.environment().remove("JANEX_HOME");
            builder.environment().put(homeVariable, root.toString());
            if (overrides[index] != null) {
                builder.environment().put("JANEX_HOME", overrides[index]);
            }
            if (index == 6) {
                builder.environment().remove(homeVariable);
            }
            if (builder.start().waitFor() != 0) {
                throw new AssertionError("Cache directory case failed: " + index);
            }
        }
    }

    /// Prevents instantiation.
    private DependenciesTest() {
    }

    /// Verifies exact Maven layouts, canonical names, and transport-policy boundaries.
    ///
    /// @param arguments optional child-process cache-directory expectation
    /// @throws Exception if mapping differs from the Host or invalid input is accepted
    public static void main(String[] arguments) throws Exception {
        if (arguments.length != 0) {
            java.lang.reflect.Method method = Dependencies.class.getDeclaredMethod("cacheDirectory");
            method.setAccessible(true);
            try {
                Object actual = method.invoke(null);
                if (!actual.toString().equals(arguments[0])) {
                    throw new AssertionError("Unexpected cache directory: " + actual);
                }
            } catch (java.lang.reflect.InvocationTargetException failure) {
                if (!arguments[0].equals("error") || !(failure.getCause() instanceof IOException)) {
                    throw failure;
                }
            }
            return;
        }
        cacheDirectories();
        address("HTTP://Example.COM:80/a/%2e%2e/library.jar?q=x", "http://example.com/library.jar?q=x", "library.jar");
        address("https://example.com/a+b.jar", "https://example.com/a+b.jar", "a+b.jar");
        address("https://example.com/e\u0301.jar", "https://example.com/e%CC%81.jar", "e\u0301.jar");
        address("http://127.1/library.jar", "http://127.0.0.1/library.jar", "library.jar");
        address("http://0x7f.0.0.01/library.jar", "http://127.0.0.1/library.jar", "library.jar");
        address("http://[0:0:0:0:0:0:0:1]/library.jar", "http://[::1]/library.jar", "library.jar");
        address("https://[2001:0DB8:0:0:1:0:0:1]:443/library.jar", "https://[2001:db8::1:0:0:1]/library.jar", "library.jar");
        address("http://[::ffff:192.0.2.1]/library.jar", "http://[::ffff:c000:201]/library.jar", "library.jar");
        address("pkg:maven/org.example/library@1.2", "https://example.com/maven/org/example/library/1.2/library-1.2.jar", "library-1.2.jar");
        address("pkg:maven/org.example/library@1.2?type=test-jar",
                "https://example.com/maven/org/example/library/1.2/library-1.2-tests.jar", "library-1.2-tests.jar");
        address("pkg:maven/org.example/library@1.2?classifier=custom&type=test-jar",
                "https://example.com/maven/org/example/library/1.2/library-1.2-custom.jar", "library-1.2-custom.jar");
        address("pkg:maven/org.example/library@1.2-20260102.030405-6",
                "https://example.com/maven/org/example/library/1.2-SNAPSHOT/library-1.2-20260102.030405-6.jar", "library-1.2-20260102.030405-6.jar");
        address("pkg:maven/g/a@1?repository_url=https:%2F%2Fmirror.example%2Frepo", "https://mirror.example/repo/g/a/1/a-1.jar", "a-1.jar");
        address("pkg:maven/g/a%2Bb@1", "https://example.com/maven/g/a+b/1/a+b-1.jar", "a+b-1.jar");
        address("pkg:maven/g/a%21b@1?classifier=x%26y",
                "https://example.com/maven/g/a!b/1/a!b-1-x&y.jar", "a!b-1-x&y.jar");
        address("pkg:maven/g/a@1?repository_url=https:%2F%2Fmirror.example%2Fa%26b",
                "https://mirror.example/a&b/g/a/1/a-1.jar", "a-1.jar");
        for (String uri : new String[]{
                "file:///a.jar", "http://user:secret@example.com/a.jar", "https://example.com/a.jar#fragment",
                "https://example.com/a.zip", "https://example.com/%2F.jar", "https://example.com/%ff.jar",
                "http://127.0.0.08/library.jar", "http://256.0.0.1/library.jar", "http://[fe80::1%25eth0]/library.jar",
                "pkg:maven/g/a", "pkg:maven/g/a@1-SNAPSHOT", "pkg:maven/g/a@LATEST", "pkg:maven/g/a@[1,2)",
                "pkg:maven/g/a@1?type=pom", "pkg:maven/g/a@1?unknown=x", "pkg:maven/g/a@1#path",
                "pkg:maven/g/a@1?type=jar&classifier=x", "pkg:maven/g/a@1?classifier=x&classifier=y",
                "pkg:maven/g/%61@1", "pkg:maven/g/a%2bb@1", "pkg:maven/g/a@1?classifier=",
                "pkg:maven/g/a!b@1", "pkg:maven/g/a@v%2Fw",
                "pkg:maven/g/a@1?repository_url=https:%2F%2Fx%2F%3Fquery"}) {
            try {
                Dependencies.address(uri, "https://example.com/maven");
            } catch (IOException expected) {
                continue;
            }
            throw new AssertionError("Invalid dependency address accepted: " + uri);
        }
    }

    /// Requires the mapped URL and filename to match fixed cross-language examples.
    private static void address(String uri, String url, String name) throws IOException {
        Dependencies.Address actual = Dependencies.address(uri, "https://example.com/maven");
        if (!actual.url.toASCIIString().equals(url) || !actual.name.equals(name)) {
            throw new AssertionError(uri + " resolved to " + actual.url + " / " + actual.name);
        }
    }
}
