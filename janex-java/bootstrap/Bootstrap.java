// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.BufferedInputStream;
import java.io.DataInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.Collections;
import java.util.Optional;

/// Loads a launch description from its own resource and invokes the application entry point.
///
/// The class targets Java 8. Module APIs are accessed only when the selected launch is modular.
public final class Bootstrap {
    /// Prevents instantiation of the launcher.
    private Bootstrap() {}

    /// Restores UTF-16 arguments and invokes the main method on the current thread.
    ///
    /// @param ignored no process arguments are needed; launch data comes from `launch.bin`
    /// @throws Throwable if launch data is invalid or the application throws
    public static void main(String[] ignored) throws Throwable {
        String moduleName;
        String className;
        boolean instanceMain;
        String[] arguments;
        InputStream resource = Bootstrap.class.getResourceAsStream("launch.bin");
        if (resource == null) {
            throw new IOException("Missing Janex launch data");
        }
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(resource))) {
            moduleName = readString(input);
            className = readString(input);
            instanceMain = input.readBoolean();
            int count = input.readInt();
            if (count < 0) {
                throw new IOException("Invalid Janex argument count");
            }
            arguments = new String[count];
            for (int i = 0; i < count; i++) {
                arguments[i] = readString(input);
            }
            if (input.read() != -1) {
                throw new IOException("Trailing Janex launch data");
            }
        }

        Class<?> application;
        if (moduleName.isEmpty()) {
            application = Class.forName(className, false, ClassLoader.getSystemClassLoader());
        } else {
            Class<?> moduleType = Class.forName("java.lang.Module");
            Object module = Class.class.getMethod("getModule").invoke(Bootstrap.class);
            if (!moduleName.equals(moduleType.getMethod("getName").invoke(module))) {
                throw new IllegalStateException("Janex bootstrap is not in the requested module");
            }
            if (className.isEmpty()) {
                Object descriptor = moduleType.getMethod("getDescriptor").invoke(module);
                Optional<?> main = (Optional<?>) Class.forName("java.lang.module.ModuleDescriptor")
                        .getMethod("mainClass").invoke(descriptor);
                if (!main.isPresent()) {
                    throw new IllegalArgumentException("Module has no main class: " + moduleName);
                }
                className = (String) main.get();
            }
            application = (Class<?>) Class.class.getMethod("forName", moduleType, String.class)
                    .invoke(null, module, className);
            if (application == null) {
                throw new ClassNotFoundException(className);
            }
            System.setProperty("jdk.module.main.class", className);
        }
        StringBuilder command = new StringBuilder();
        if (!moduleName.isEmpty()) {
            command.append(moduleName).append('/');
        }
        command.append(className);
        for (String argument : arguments) {
            command.append(' ').append(argument);
        }
        System.setProperty("sun.java.command", command.toString());

        Method main = findMain(application, instanceMain, String[].class);
        if (main == null && instanceMain) {
            main = findMain(application, true);
        }
        if (main == null) {
            throw new NoSuchMethodException("No eligible main method in " + className);
        }
        MethodHandle handle;
        try {
            MethodHandles.Lookup lookup = (MethodHandles.Lookup) MethodHandles.class
                    .getMethod("privateLookupIn", Class.class, MethodHandles.Lookup.class)
                    .invoke(null, application, MethodHandles.lookup());
            handle = lookup.unreflect(main).asFixedArity();
        } catch (NoSuchMethodException java8) {
            main.setAccessible(true);
            handle = MethodHandles.lookup().unreflect(main).asFixedArity();
        }
        if (!Modifier.isStatic(main.getModifiers())) {
            Constructor<?> constructor = application.getDeclaredConstructor();
            if (Modifier.isPrivate(constructor.getModifiers())) {
                throw new IllegalAccessException("Main class constructor is private");
            }
            constructor.setAccessible(true);
            try {
                handle = handle.bindTo(constructor.newInstance());
            } catch (InvocationTargetException exception) {
                throw exception.getCause();
            }
        }
        handle.invokeWithArguments(main.getParameterCount() == 0
                ? Collections.emptyList() : Collections.singletonList(arguments));
    }

    /// Finds a main method, retaining inheritance and package-access restrictions.
    ///
    /// @param application the class named by the launch description
    /// @param instanceMain whether non-private instance and no-argument main methods are allowed
    /// @param parameters the candidate main signature
    /// @return an eligible main method, or `null`
    private static Method findMain(Class<?> application, boolean instanceMain, Class<?>... parameters) {
        if (!instanceMain) {
            try {
                Method method = application.getMethod("main", parameters);
                return Modifier.isStatic(method.getModifiers()) && method.getReturnType() == void.class
                        ? method : null;
            } catch (NoSuchMethodException absent) {
                return null;
            }
        }
        for (Class<?> type = application; type != null; type = type.getSuperclass()) {
            try {
                Method method = type.getDeclaredMethod("main", parameters);
                int modifiers = method.getModifiers();
                boolean inherited = true;
                if (!Modifier.isPublic(modifiers) && !Modifier.isProtected(modifiers)) {
                    for (Class<?> child = application; child != type; child = child.getSuperclass()) {
                        if (!samePackage(child, type)) {
                            inherited = false;
                            break;
                        }
                    }
                }
                if (!Modifier.isPrivate(modifiers) && inherited && method.getReturnType() == void.class) {
                    return method;
                }
            } catch (NoSuchMethodException absent) {
                // Continue with inherited methods.
            }
        }
        try {
            Method method = application.getMethod("main", parameters);
            return method.getReturnType() == void.class ? method : null;
        } catch (NoSuchMethodException absent) {
            return null;
        }
    }

    /// Tests runtime package identity for inheritance of package-access methods.
    ///
    /// @param first a class in the candidate runtime package
    /// @param second another class whose package is compared
    /// @return whether the package names and defining loaders match
    private static boolean samePackage(Class<?> first, Class<?> second) {
        String a = first.getName();
        String b = second.getName();
        return first.getClassLoader() == second.getClassLoader()
                && a.substring(0, a.lastIndexOf('.') + 1).equals(b.substring(0, b.lastIndexOf('.') + 1));
    }

    /// Reads a counted sequence of UTF-16 code units without charset conversion.
    ///
    /// @param input launch data positioned at a string length
    /// @return the decoded string, including any unpaired surrogate code units
    /// @throws IOException if the length is invalid or the string is truncated
    private static String readString(DataInputStream input) throws IOException {
        int length = input.readInt();
        if (length < 0) {
            throw new IOException("Invalid Janex string length");
        }
        char[] characters = new char[length];
        for (int i = 0; i < length; i++) {
            characters[i] = input.readChar();
        }
        return new String(characters);
    }
}
