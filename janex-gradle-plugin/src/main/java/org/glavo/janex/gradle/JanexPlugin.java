// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.gradle;

import java.util.Collections;
import java.util.Map;

import org.gradle.api.Plugin;
import org.gradle.api.Project;
import org.gradle.api.plugins.JavaApplication;
import org.gradle.api.plugins.JavaPlugin;
import org.gradle.api.provider.Provider;
import org.gradle.api.tasks.TaskProvider;
import org.gradle.api.tasks.SourceSet;
import org.gradle.api.tasks.SourceSetContainer;

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
        extension.getInputDirectories().from(project.getExtensions().getByType(SourceSetContainer.class)
                .named("main").map(SourceSet::getOutput));
        extension.getSourceName().convention(project.getName() + ".jar");
        extension.getManifestAttributes().convention(Map.of("Manifest-Version", "1.0"));
        extension.getApplicationId().convention("main");
        extension.getJvmOptions().convention(Collections.emptyList());
        extension.getArguments().convention(Collections.emptyList());
        extension.getWithLauncher().convention(true);
        extension.getCompression().convention(true);
        extension.getCompressionLevel().convention(3);
        extension.getTransformClassfiles().convention(true);
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
            task.getInputDirectories().from(extension.getInputDirectories());
            task.getSourceName().convention(extension.getSourceName());
            task.getManifestAttributes().convention(extension.getManifestAttributes());
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
            task.getCompressionLevel().convention(extension.getCompressionLevel());
            task.getTransformClassfiles().convention(extension.getTransformClassfiles());
            task.getSigning().convention(extension.getSigning());
            task.getMinimization().convention(extension.getMinimization());
            task.getResources().convention(extension.getResources());
            task.getNativeLauncher().convention(extension.getNativeLauncher());
            task.getNativeLaunchMode().convention(extension.getNativeLaunchMode());
            task.getOutputFile().convention(extension.getOutputFile());
        });
        project.getTasks().named("assemble").configure(task -> task.dependsOn(pack));
    }
}
