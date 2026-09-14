// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.gradle;

import java.util.Collections;

import org.gradle.api.Plugin;
import org.gradle.api.Project;
import org.gradle.api.plugins.JavaApplication;
import org.gradle.api.plugins.JavaPlugin;
import org.gradle.api.provider.Provider;
import org.gradle.api.tasks.TaskProvider;
import org.gradle.api.tasks.bundling.Jar;

/// Applies the Java plugin and registers the `janex` extension and `janexPack` task.
/// The package participates in `assemble`. Applying the application plugin supplies
/// entry-point and JVM-argument conventions without replacing explicit Janex settings.
public final class JanexPlugin implements Plugin<Project> {
    /// Creates a plugin instance.
    public JanexPlugin() {
    }

    /// Configures packaging for the supplied project without resolving its dependencies.
    ///
    /// @param project the project receiving the plugin
    @Override
    public void apply(Project project) {
        project.getPluginManager().apply(JavaPlugin.class);
        JanexExtension extension = project.getExtensions().create("janex", JanexExtension.class);
        extension.getSource().convention(project.getTasks().named("jar", Jar.class).flatMap(Jar::getArchiveFile));
        extension.getApplicationId().convention("main");
        extension.getJvmOptions().convention(Collections.emptyList());
        extension.getArguments().convention(Collections.emptyList());
        extension.getWithLauncher().convention(true);
        extension.getCompression().convention(true);
        extension.getNativeLaunchMode().convention("bootstrap");
        extension.getOutputFile().convention(
                project.getLayout().getBuildDirectory().file("distributions/" + project.getName() + ".janex"));

        Provider<Boolean> modular = extension.getMainModule().map(name -> true).orElse(false);
        var dependencies = project.getConfigurations().getByName("runtimeClasspath").getElements();
        extension.getClassPath().from(modular.zip(dependencies,
                (isModular, files) -> isModular ? Collections.emptySet() : files));
        extension.getModulePath().from(modular.zip(dependencies,
                (isModular, files) -> isModular ? files : Collections.emptySet()));

        project.getPluginManager().withPlugin("application", ignored -> {
            JavaApplication application = project.getExtensions().getByType(JavaApplication.class);
            extension.getMainClass().convention(application.getMainClass());
            extension.getMainModule().convention(application.getMainModule());
            extension.getJvmOptions().convention(project.provider(application::getApplicationDefaultJvmArgs));
        });

        TaskProvider<JanexPack> pack = project.getTasks().register("janexPack", JanexPack.class, task -> {
            task.setGroup("distribution");
            task.setDescription("Packages the application and its runtime dependencies as a Janex file.");
            task.getSource().convention(extension.getSource());
            task.getClassPath().from(extension.getClassPath());
            task.getModulePath().from(extension.getModulePath());
            task.getMainClass().convention(extension.getMainClass());
            task.getMainModule().convention(extension.getMainModule());
            task.getApplicationId().convention(extension.getApplicationId());
            task.getJvmOptions().convention(extension.getJvmOptions());
            task.getArguments().convention(extension.getArguments());
            task.getJavaVersion().convention(extension.getJavaVersion());
            task.getWithLauncher().convention(extension.getWithLauncher());
            task.getCompression().convention(extension.getCompression());
            task.getNativeLauncher().convention(extension.getNativeLauncher());
            task.getNativeLaunchMode().convention(extension.getNativeLaunchMode());
            task.getOutputFile().convention(extension.getOutputFile());
        });
        project.getTasks().named("assemble").configure(task -> task.dependsOn(pack));
    }
}
