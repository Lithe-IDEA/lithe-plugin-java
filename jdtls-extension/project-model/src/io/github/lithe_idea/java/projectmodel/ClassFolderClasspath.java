package io.github.lithe_idea.java.projectmodel;

import java.io.File;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

import org.eclipse.core.runtime.CoreException;
import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.IProgressMonitor;
import org.eclipse.core.runtime.Path;
import org.eclipse.jdt.core.IClasspathContainer;
import org.eclipse.jdt.core.IClasspathEntry;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.JavaCore;
import org.eclipse.jdt.ls.core.internal.ProjectUtils;

/** Adds IDEA class directories to JDT's own invisible project, whose metadata lives under the server workspace. */
final class ClassFolderClasspath {
	static final IPath ID = new Path("io.github.lithe_idea.java.projectmodel.IDEA_CLASS_FOLDERS");
	static final String DESCRIPTION = "IntelliJ IDEA class folders";

	static void refresh(ProjectModelSettings settings, IProgressMonitor monitor) {
		for (IJavaProject project : ProjectUtils.getJavaProjects()) {
			if (monitor.isCanceled()) {
				return;
			}
			if (!ProjectUtils.isUnmanagedFolder(project.getProject())) {
				continue;
			}
			try {
				List<IClasspathEntry> raw = new ArrayList<>(Arrays.asList(project.getRawClasspath()));
				boolean present = raw.stream().anyMatch(ClassFolderClasspath::isOwned);
				boolean active = applies(project, settings);
				if (active) {
					IClasspathEntry[] entries = entries(settings);
					IClasspathContainer current = present ? JavaCore.getClasspathContainer(ID, project) : null;
					if (current == null || !Arrays.equals(current.getClasspathEntries(), entries)) {
						bind(project, entries, monitor);
					}
					if (!present) {
						raw.add(JavaCore.newContainerEntry(ID));
					}
				} else if (present) {
					raw.removeIf(ClassFolderClasspath::isOwned);
				}
				if (present != active) {
					project.setRawClasspath(raw.toArray(IClasspathEntry[]::new), monitor);
				}
			} catch (CoreException e) {
				ProjectModelLog.error("Cannot update IDEA class folders for " + project.getElementName(), e);
			}
		}
	}

	static void bind(IJavaProject project, ProjectModelSettings settings, IProgressMonitor monitor) throws CoreException {
		bind(project, applies(project, settings) ? entries(settings) : new IClasspathEntry[0], monitor);
	}

	private static IClasspathEntry[] entries(ProjectModelSettings settings) {
		return settings.classFolders.stream()
				.map(File::new)
				.filter(File::isDirectory)
				.map(folder -> JavaCore.newLibraryEntry(Path.fromOSString(folder.getPath()), null, null))
				.toArray(IClasspathEntry[]::new);
	}

	private static void bind(IJavaProject project, IClasspathEntry[] entries, IProgressMonitor monitor) throws CoreException {
		IClasspathContainer container = new LitheContainer(ID, DESCRIPTION, entries);
		JavaCore.setClasspathContainer(ID, new IJavaProject[] { project }, new IClasspathContainer[] { container }, monitor);
	}

	private static boolean applies(IJavaProject project, ProjectModelSettings settings) {
		IPath folder = ProjectUtils.getProjectRealFolder(project.getProject());
		return !settings.classFolders.isEmpty()
				&& ProjectModelSettings.sameDirectory(folder, settings.classFolderWorkspace);
	}

	private static boolean isOwned(IClasspathEntry entry) {
		return entry.getEntryKind() == IClasspathEntry.CPE_CONTAINER && ID.equals(entry.getPath());
	}
}
