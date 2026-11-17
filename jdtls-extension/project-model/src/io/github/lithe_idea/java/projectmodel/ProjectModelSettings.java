package io.github.lithe_idea.java.projectmodel;

import java.io.File;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Objects;

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

	private ProjectModelSettings(boolean enabled, String serverRuntimeHome, List<String> serverRuntimeLibraries, String serverRuntimeName) {
		this.enabled = enabled;
		this.serverRuntimeHome = serverRuntimeHome;
		this.serverRuntimeLibraries = serverRuntimeLibraries;
		this.serverRuntimeName = serverRuntimeName;
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
			return new ProjectModelSettings(true, "", List.of(), "");
		}
		boolean enabled = !Boolean.FALSE.equals(map.get("enabled"));
		List<String> libraries = new ArrayList<>();
		if (map.get("serverRuntimeLibraries") instanceof List<?> list) {
			for (Object item : list) {
				if (libraries.size() >= MAX_LIBRARIES) {
					break;
				}
				if (item instanceof String path && new File(path).isAbsolute()) {
					libraries.add(path);
				}
			}
		}
		String name = map.get("serverRuntimeName") instanceof String text ? text : "";
		String home = map.get("serverRuntimeHome") instanceof String text && new File(text).isAbsolute() ? text : "";
		return new ProjectModelSettings(enabled, home, Collections.unmodifiableList(libraries), name);
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

	@Override
	public boolean equals(Object other) {
		return other instanceof ProjectModelSettings that && enabled == that.enabled
				&& serverRuntimeHome.equals(that.serverRuntimeHome)
				&& serverRuntimeLibraries.equals(that.serverRuntimeLibraries)
				&& serverRuntimeName.equals(that.serverRuntimeName);
	}

	@Override
	public int hashCode() {
		return Objects.hash(enabled, serverRuntimeHome, serverRuntimeLibraries, serverRuntimeName);
	}
}
