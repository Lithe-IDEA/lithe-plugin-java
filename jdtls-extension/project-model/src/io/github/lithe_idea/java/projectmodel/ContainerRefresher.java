package io.github.lithe_idea.java.projectmodel;

import java.util.List;
import java.util.Map;

import org.eclipse.core.resources.IResource;
import org.eclipse.core.resources.IResourceChangeEvent;
import org.eclipse.core.resources.IResourceChangeListener;
import org.eclipse.core.resources.IResourceDelta;
import org.eclipse.core.resources.IResourceDeltaVisitor;
import org.eclipse.core.runtime.CoreException;
import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.IProgressMonitor;
import org.eclipse.core.runtime.IStatus;
import org.eclipse.core.runtime.Status;
import org.eclipse.core.runtime.jobs.Job;
import org.eclipse.jdt.core.IClasspathContainer;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.JavaCore;
import org.eclipse.jdt.ls.core.internal.preferences.IPreferencesChangeListener;
import org.eclipse.jdt.ls.core.internal.preferences.Preferences;

/**
 * Recomputes every container this bundle owns when its configuration changes
 * or a file it reads changes (a JAR dropped into WEB-INF/lib, an edited
 * component or manifest). One job coalesces bursts of events.
 */
final class ContainerRefresher extends Job implements IPreferencesChangeListener, IResourceChangeListener {
	/** Coalesces a burst of file events, such as copying many JARs, into one refresh. */
	static final long DELAY_MILLISECONDS = 500;

	ContainerRefresher() {
		super("Refresh Lithe project model containers");
		setSystem(true);
	}

	@Override
	public void preferencesChange(Preferences oldPreferences, Preferences newPreferences) {
		ProjectModelSettings before = ProjectModelSettings.from(oldPreferences);
		ProjectModelSettings after = ProjectModelSettings.from(newPreferences);
		if (!before.equals(after)) {
			ServletHomeVariableInitializer.apply(after, ServletHomeVariableInitializer.VARIABLES);
			schedule(0);
		}
	}

	@Override
	public void resourceChanged(IResourceChangeEvent event) {
		IResourceDelta delta = event.getDelta();
		if (delta == null) {
			return;
		}
		boolean[] relevant = { false };
		try {
			delta.accept((IResourceDeltaVisitor) child -> {
				if (relevant[0]) {
					return false;
				}
				IResource resource = child.getResource();
				if (resource.getType() == IResource.FILE && isModelInput(resource.getProjectRelativePath())) {
					relevant[0] = true;
				}
				return true;
			});
		} catch (CoreException e) {
			ProjectModelLog.error("Cannot inspect a resource change", e);
		}
		if (relevant[0] && !ContainerResolver.ownedContainers().isEmpty()) {
			schedule(DELAY_MILLISECONDS);
		}
	}

	/** Whether a project file feeds a container: a library or a module descriptor. */
	static boolean isModelInput(IPath path) {
		String name = path.lastSegment();
		if (name == null) {
			return false;
		}
		String lower = name.toLowerCase(java.util.Locale.ROOT);
		if (lower.endsWith(".jar") || lower.endsWith(".zip")) {
			return true;
		}
		return name.equals("org.eclipse.wst.common.component") || name.equals(WebModules.MYECLIPSE_METADATA)
				|| name.equals("application.xml") || name.equals("MANIFEST.MF");
	}

	@Override
	protected IStatus run(IProgressMonitor monitor) {
		ProjectModelSettings settings = ProjectModelSettings.current();
		for (Map.Entry<IPath, List<IJavaProject>> owned : ContainerResolver.ownedContainers().entrySet()) {
			if (monitor.isCanceled()) {
				return Status.CANCEL_STATUS;
			}
			List<IJavaProject> projects = owned.getValue();
			IClasspathContainer[] containers = new IClasspathContainer[projects.size()];
			for (int index = 0; index < containers.length; index++) {
				// A null container unbinds it, which is what disabling means.
				containers[index] = settings.enabled ? ContainerResolver.resolve(owned.getKey(), projects.get(index), settings) : null;
			}
			try {
				JavaCore.setClasspathContainer(owned.getKey(), projects.toArray(IJavaProject[]::new), containers, monitor);
			} catch (CoreException e) {
				ProjectModelLog.error("Cannot refresh " + owned.getKey(), e);
			}
		}
		return Status.OK_STATUS;
	}
}
