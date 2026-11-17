package io.github.lithe_idea.java.projectmodel;

import java.util.List;

import org.eclipse.core.runtime.Path;
import org.eclipse.jdt.core.ClasspathVariableInitializer;
import org.eclipse.jdt.core.JavaCore;
import org.eclipse.jdt.core.JavaModelException;

/**
 * Binds the classpath variables legacy Eclipse projects use for the servlet
 * container ({@code TOMCAT_HOME/lib/servlet-api.jar}) to the folder the backend
 * configured. Without one the variables stay unbound, as without this bundle.
 */
public final class ServletHomeVariableInitializer extends ClasspathVariableInitializer {
	static final List<String> VARIABLES = List.of("TOMCAT_HOME", "CATALINA_HOME");

	@Override
	public void initialize(String variable) {
		apply(ProjectModelSettings.current(), List.of(variable));
	}

	/** Sets {@code variables} from {@code settings}, or removes them. */
	static void apply(ProjectModelSettings settings, List<String> variables) {
		boolean bound = settings.enabled && !settings.serverRuntimeHome.isEmpty();
		try {
			for (String variable : variables) {
				if (bound) {
					JavaCore.setClasspathVariable(variable, new Path(settings.serverRuntimeHome), null);
				} else if (JavaCore.getClasspathVariable(variable) != null) {
					JavaCore.removeClasspathVariable(variable, null);
				}
			}
		} catch (JavaModelException e) {
			ProjectModelLog.error("Cannot update the servlet container classpath variables", e);
		}
	}
}
