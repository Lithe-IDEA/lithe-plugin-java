package io.github.lithe_idea.java.projectmodel;

import java.io.File;
import java.io.IOException;
import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;

import org.eclipse.jdt.ls.core.internal.JavaLanguageServerPlugin;
import org.eclipse.jdt.ls.core.internal.preferences.PreferenceManager;
import org.eclipse.jdt.ls.core.internal.preferences.Preferences;

/**
 * The part of the workspace configuration the plugin backend writes for this
 * bundle: {@code java.lithe.projectModel = {enabled, serverRuntimeHome,
 * serverRuntimeLibraries, serverRuntimeName}}. The backend owns detection of the servlet container; the
 * bundle only consumes the absolute library paths it was given.
 */
final class ProjectModelSettings {
	/** Bounds a malformed or hostile configuration; a real container has far fewer. */
	static final int MAX_LIBRARIES = 1024;

	final boolean enabled;
	/** Absolute servlet container folder, or empty when none was found. */
	final String serverRuntimeHome;
	final List<String> serverRuntimeLibraries;
	final String serverRuntimeName;
	final String classFolderWorkspace;
	final List<String> classFolders;
	final Map<String, List<String>> sourceRoles;
	final Optional<List<String>> workspaceSourcePaths;
	final Optional<String> workspaceOutputPath;

	private ProjectModelSettings(boolean enabled, String serverRuntimeHome, List<String> serverRuntimeLibraries, String serverRuntimeName,
			String classFolderWorkspace, List<String> classFolders, Map<String, List<String>> sourceRoles,
			Optional<List<String>> workspaceSourcePaths, Optional<String> workspaceOutputPath) {
		this.enabled = enabled;
		this.serverRuntimeHome = serverRuntimeHome;
		this.serverRuntimeLibraries = serverRuntimeLibraries;
		this.serverRuntimeName = serverRuntimeName;
		this.classFolderWorkspace = classFolderWorkspace;
		this.classFolders = classFolders;
		this.sourceRoles = sourceRoles;
		this.workspaceSourcePaths = workspaceSourcePaths;
		this.workspaceOutputPath = workspaceOutputPath;
	}

	/** The settings of the current JDT LS preferences; defaults when absent. */
	static ProjectModelSettings current() {
		PreferenceManager manager = JavaLanguageServerPlugin.getPreferencesManager();
		return manager == null ? from(null) : from(manager.getPreferences());
	}

	static ProjectModelSettings from(Preferences preferences) {
		Object raw = preferences == null ? null : lookup(preferences.asMap(), "java", "lithe", "projectModel");
		if (!(raw instanceof Map<?, ?> map)) {
			// Without a configuration from the backend (an older backend, or a
			// JDT LS started by another client) the containers still resolve
			// from project files; only the servlet container stays empty.
			return new ProjectModelSettings(true, "", List.of(), "", "", List.of(), Map.of(), Optional.empty(), Optional.empty());
		}
		boolean enabled = !Boolean.FALSE.equals(map.get("enabled"));
		String name = map.get("serverRuntimeName") instanceof String text ? text : "";
		String home = map.get("serverRuntimeHome") instanceof String text && new File(text).isAbsolute() ? text : "";
		String workspace = map.get("classFolderWorkspace") instanceof String text && new File(text).isAbsolute() ? text : "";
		Map<String, List<String>> roles = new java.util.HashMap<>();
		if (map.get("sourceRoles") instanceof Map<?, ?> sourceRoles) {
			for (String role : List.of("test", "resource", "test-resource", "excluded")) {
				List<String> paths = absolutePaths(sourceRoles.get(role));
				if (!paths.isEmpty()) {
					roles.put(role, paths);
				}
			}
		}
		Optional<List<String>> sources = map.get("workspaceSourcePaths") instanceof List<?> paths
				? Optional.of(paths.stream().filter(String.class::isInstance).map(String.class::cast)
						.limit(MAX_LIBRARIES).toList())
				: Optional.empty();
		Optional<String> output = map.get("workspaceOutputPath") instanceof String path ? Optional.of(path) : Optional.empty();
		return new ProjectModelSettings(enabled, home, absolutePaths(map.get("serverRuntimeLibraries")), name,
				workspace, absolutePaths(map.get("classFolders")), Map.copyOf(roles), sources, output);
	}

	private static List<String> absolutePaths(Object value) {
		List<String> libraries = new ArrayList<>();
		if (value instanceof List<?> list) {
			for (Object item : list) {
				if (libraries.size() >= MAX_LIBRARIES) {
					break;
				}
				if (item instanceof String path && new File(path).isAbsolute()) {
					libraries.add(path);
				}
			}
		}
		return Collections.unmodifiableList(libraries);
	}

	/**
	 * Reads {@code java.lithe.projectModel} whether the client sent it nested
	 * or with dotted keys, the two shapes JDT LS itself accepts.
	 */
	private static Object lookup(Map<String, Object> configuration, String... keys) {
		if (configuration == null) {
			return null;
		}
		Object dotted = configuration.get(String.join(".", keys));
		if (dotted != null) {
			return dotted;
		}
		Object current = configuration;
		for (String key : keys) {
			if (!(current instanceof Map<?, ?> map)) {
				return null;
			}
			current = map.get(key);
		}
		return current;
	}

	static Path realPath(Path path) {
		try {
			return path.toRealPath();
		} catch (IOException e) {
			return path.toAbsolutePath().normalize();
		}
	}

	static Path realPath(String path) {
		try {
			return realPath(Path.of(path));
		} catch (InvalidPathException e) {
			return null;
		}
	}

	static boolean sameDirectory(org.eclipse.core.runtime.IPath folder, String other) {
		if (folder == null || other == null || other.isEmpty()) {
			return false;
		}
		Path otherPath = realPath(other);
		return otherPath != null && realPath(folder.toFile().toPath()).equals(otherPath);
	}

	@Override
	public boolean equals(Object other) {
		return other instanceof ProjectModelSettings that && enabled == that.enabled
				&& serverRuntimeHome.equals(that.serverRuntimeHome)
				&& serverRuntimeLibraries.equals(that.serverRuntimeLibraries)
				&& serverRuntimeName.equals(that.serverRuntimeName)
				&& classFolderWorkspace.equals(that.classFolderWorkspace)
				&& classFolders.equals(that.classFolders)
				&& sourceRoles.equals(that.sourceRoles)
				&& workspaceSourcePaths.equals(that.workspaceSourcePaths)
				&& workspaceOutputPath.equals(that.workspaceOutputPath);
	}

	@Override
	public int hashCode() {
		return Objects.hash(enabled, serverRuntimeHome, serverRuntimeLibraries, serverRuntimeName,
				classFolderWorkspace, classFolders, sourceRoles, workspaceSourcePaths, workspaceOutputPath);
	}
}
