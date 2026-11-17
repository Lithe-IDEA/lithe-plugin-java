package io.github.lithe_idea.java.projectmodel;

import org.eclipse.jdt.ls.core.internal.JavaLanguageServerPlugin;

/** Routes failures into the JDT LS log, which the plugin configures and bounds. */
final class ProjectModelLog {
	private ProjectModelLog() {
	}

	static void error(String message, Throwable error) {
		JavaLanguageServerPlugin.logException("[Lithe project model] " + message, error);
	}
}
