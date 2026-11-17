package io.github.lithe_idea.java.projectmodel;

import org.eclipse.core.runtime.IPath;
import org.eclipse.jdt.core.IClasspathContainer;
import org.eclipse.jdt.core.IClasspathEntry;

/** An immutable container value computed by {@link ContainerResolver}. */
record LitheContainer(IPath path, String description, IClasspathEntry[] entries) implements IClasspathContainer {
	@Override
	public IClasspathEntry[] getClasspathEntries() {
		return entries.clone();
	}

	@Override
	public String getDescription() {
		return description;
	}

	@Override
	public int getKind() {
		return K_APPLICATION;
	}

	@Override
	public IPath getPath() {
		return path;
	}
}
