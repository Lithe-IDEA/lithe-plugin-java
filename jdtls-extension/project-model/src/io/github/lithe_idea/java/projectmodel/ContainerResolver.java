package io.github.lithe_idea.java.projectmodel;

import java.io.File;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import org.eclipse.core.resources.IProject;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.Path;
import org.eclipse.jdt.core.IClasspathEntry;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.JavaCore;

/**
 * Computes the value of one container for one project from project files and
 * the configured servlet container. Pure apart from bounded file reads, so the
 * initializer, the refresh job and the describe command share one answer.
 */
final class ContainerResolver {
	private ContainerResolver() {
	}

	/** The container of {@code path} for {@code project}, never null for a known kind. */
	static LitheContainer resolve(IPath path, IJavaProject project, ProjectModelSettings settings) {
		ContainerKind kind = ContainerKind.of(path);
		File directory = location(project.getProject());
		Map<String, IClasspathEntry> entries = new LinkedHashMap<>();
		if (kind != null && directory != null) {
			switch (kind) {
			case WEB_APP -> webAppEntries(project.getProject(), directory, entries);
			case EAR -> earEntries(project.getProject(), directory, entries);
			case SERVER_RUNTIME, JAVA_EE -> {
				for (String library : settings.serverRuntimeLibraries) {
					addLibrary(new File(library), entries);
				}
			}
			}
		}
		return new LitheContainer(path, description(path, kind, settings), entries.values().toArray(IClasspathEntry[]::new));
	}

	static String description(IPath path, ContainerKind kind, ProjectModelSettings settings) {
		if (kind == null) {
			return path.toString();
		}
		return switch (kind) {
		case WEB_APP -> "Web App Libraries";
		case EAR -> "EAR Libraries";
		case SERVER_RUNTIME -> "Server Runtime [" + (path.segmentCount() > 2 ? path.segment(2) : path.lastSegment()) + "]";
		case JAVA_EE -> "Java EE Libraries [" + path.lastSegment() + "]";
		};
	}

	/** WEB-INF/lib of the web root, then the utility modules deployed there. */
	private static void webAppEntries(IProject project, File directory, Map<String, IClasspathEntry> entries) {
		File webRoot = WebModules.webRoot(directory);
		if (webRoot != null) {
			for (File library : WebModules.libraries(new File(webRoot, "WEB-INF/lib"))) {
				addLibrary(library, entries);
			}
		}
		WebModules.Component component = WebModules.readComponent(directory);
		if (component == null) {
			return;
		}
		for (WebModules.DependentModule module : component.dependents()) {
			if (module.deployedTo("/WEB-INF/lib")) {
				addDependent(project, module, entries);
			}
		}
	}

	/**
	 * The library directory of every EAR in the workspace that bundles this
	 * module, the modules those EARs deploy there, and the module's manifest
	 * Class-Path resolved against the EAR's content.
	 */
	private static void earEntries(IProject project, File directory, Map<String, IClasspathEntry> entries) {
		List<String> classPath = WebModules.manifestClassPath(WebModules.moduleRoot(directory));
		for (IProject candidate : ResourcesPlugin.getWorkspace().getRoot().getProjects()) {
			File earDirectory = location(candidate);
			if (candidate.equals(project) || earDirectory == null) {
				continue;
			}
			WebModules.Component ear = WebModules.readComponent(earDirectory);
			if (ear == null || ear.dependents().stream().noneMatch(module -> project.getName().equals(module.projectName()))) {
				continue;
			}
			File earRoot = WebModules.earRoot(earDirectory, ear);
			String libraryDirectory = WebModules.earLibraryDirectory(earRoot);
			for (File library : WebModules.libraries(new File(earRoot, libraryDirectory))) {
				addLibrary(library, entries);
			}
			for (WebModules.DependentModule module : ear.dependents()) {
				if (module.deployedTo(libraryDirectory) && !project.getName().equals(module.projectName())) {
					addDependent(candidate, module, entries);
				}
			}
			for (String entry : classPath) {
				File library = new File(earRoot, entry);
				if (library.isFile()) {
					addLibrary(library, entries);
					continue;
				}
				String archive = new Path(entry).lastSegment();
				for (WebModules.DependentModule module : ear.dependents()) {
					if (archive != null && archive.equals(module.archiveName()) && !project.getName().equals(module.projectName())) {
						addDependent(candidate, module, entries);
					}
				}
			}
		}
	}

	/** A dependent module: a workspace project, a library path or a variable path. */
	private static void addDependent(IProject owner, WebModules.DependentModule module, Map<String, IClasspathEntry> entries) {
		String projectName = module.projectName();
		if (projectName != null) {
			IProject referenced = ResourcesPlugin.getWorkspace().getRoot().getProject(projectName);
			if (referenced.isAccessible() && !referenced.equals(owner)) {
				IPath path = referenced.getFullPath();
				entries.putIfAbsent("project:" + path, JavaCore.newProjectEntry(path, false));
			}
			return;
		}
		String library = module.libraryPath();
		if (library != null) {
			addLibrary(libraryFile(library), entries);
			return;
		}
		String[] variable = module.variablePath();
		if (variable != null) {
			IPath root = JavaCore.getClasspathVariable(variable[0]);
			if (root != null) {
				addLibrary(root.append(variable[1]).toFile(), entries);
			}
		}
	}

	/** A WTP library path: {@code /Project/rest} inside the workspace, else a file path. */
	private static File libraryFile(String value) {
		IPath path = new Path(value);
		if (!path.isEmpty() && path.getDevice() == null && path.segmentCount() > 1) {
			IProject project = ResourcesPlugin.getWorkspace().getRoot().getProject(path.segment(0));
			File directory = location(project);
			if (directory != null) {
				return new File(directory, path.removeFirstSegments(1).toOSString());
			}
		}
		return path.toFile();
	}

	private static void addLibrary(File library, Map<String, IClasspathEntry> entries) {
		if (library == null || !library.isFile()) {
			return;
		}
		File absolute = library.getAbsoluteFile();
		String key = "library:" + absolute.getPath();
		if (entries.containsKey(key)) {
			return;
		}
		File source = WebModules.sourceAttachment(absolute);
		entries.put(key, JavaCore.newLibraryEntry(new Path(absolute.getPath()), source == null ? null : new Path(source.getPath()), null));
	}

	static File location(IProject project) {
		if (project == null || !project.isAccessible() || project.getLocation() == null) {
			return null;
		}
		return project.getLocation().toFile();
	}

	/** Every raw container entry of a kind this bundle resolves, per project. */
	static Map<IPath, List<IJavaProject>> ownedContainers() {
		Map<IPath, List<IJavaProject>> owned = new LinkedHashMap<>();
		for (IProject project : ResourcesPlugin.getWorkspace().getRoot().getProjects()) {
			if (!project.isAccessible()) {
				continue;
			}
			IJavaProject javaProject = JavaCore.create(project);
			try {
				if (!javaProject.exists()) {
					continue;
				}
				for (IClasspathEntry entry : javaProject.getRawClasspath()) {
					if (entry.getEntryKind() == IClasspathEntry.CPE_CONTAINER && ContainerKind.of(entry.getPath()) != null) {
						owned.computeIfAbsent(entry.getPath(), key -> new ArrayList<>()).add(javaProject);
					}
				}
			} catch (Exception e) {
				// A project whose classpath cannot be read keeps its containers.
			}
		}
		return owned;
	}
}
