package io.github.lithe_idea.java.projectmodel;

import org.eclipse.core.runtime.CoreException;
import org.eclipse.core.runtime.IPath;
import org.eclipse.jdt.core.ClasspathContainerInitializer;
import org.eclipse.jdt.core.IClasspathContainer;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.JavaCore;

/**
 * JDT's entry point for the containers in plugin.xml. When the backend turned
 * project model detection off, the container is left unbound, exactly as JDT
 * LS behaves without this bundle.
 */
public final class LitheContainerInitializer extends ClasspathContainerInitializer {
	@Override
	public void initialize(IPath containerPath, IJavaProject project) throws CoreException {
		ProjectModelSettings settings = ProjectModelSettings.current();
		if (ClassFolderClasspath.ID.equals(containerPath)) {
			ClassFolderClasspath.bind(project, settings, null);
			return;
		}
		if (!settings.enabled || ContainerKind.of(containerPath) == null) {
			return;
		}
		IClasspathContainer container = ContainerResolver.resolve(containerPath, project, settings);
		JavaCore.setClasspathContainer(containerPath, new IJavaProject[] { project }, new IClasspathContainer[] { container }, null);
	}

	@Override
	public String getDescription(IPath containerPath, IJavaProject project) {
		if (ClassFolderClasspath.ID.equals(containerPath)) {
			return ClassFolderClasspath.DESCRIPTION;
		}
		return ContainerResolver.description(containerPath, ContainerKind.of(containerPath), ProjectModelSettings.current());
	}

	@Override
	public boolean canUpdateClasspathContainer(IPath containerPath, IJavaProject project) {
		// The value is derived from project files; editing it in place would
		// create a second source of truth.
		return false;
	}
}
