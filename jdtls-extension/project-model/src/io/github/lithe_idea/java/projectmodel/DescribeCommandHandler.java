package io.github.lithe_idea.java.projectmodel;

import java.io.File;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import org.eclipse.core.resources.IProject;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.core.runtime.IProgressMonitor;
import org.eclipse.jdt.core.IClasspathContainer;
import org.eclipse.jdt.core.IClasspathEntry;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.JavaCore;
import org.eclipse.jdt.ls.core.internal.IDelegateCommandHandler;

/**
 * {@code lithe.java.projectModel.describe}: what JDT actually made of each
 * project's container, variable and library entries, for the plugin's
 * settings page. Read-only; asking resolves a container JDT has not resolved
 * yet, as any classpath query would.
 *
 * <p>Result: {@code {version: 1, enabled, serverRuntimeName, projects: [{name,
 * location, entries: [{kind, path, containerKind?, description?, status,
 * libraries, projects}]}]}}. {@code status} is {@code resolved}, {@code empty},
 * {@code unbound}, {@code missing} or {@code disabled}. Library entries are
 * listed only when their file is missing.
 */
public final class DescribeCommandHandler implements IDelegateCommandHandler {
	static final String COMMAND = "lithe.java.projectModel.describe";
	/** Bounds the answer; the settings page shows at most this many rows per project. */
	static final int MAX_ENTRIES_PER_PROJECT = 128;
	static final int MAX_PROJECTS = 1024;

	@Override
	public Object executeCommand(String commandId, List<Object> arguments, IProgressMonitor monitor) throws Exception {
		if (!COMMAND.equals(commandId)) {
			throw new UnsupportedOperationException(commandId);
		}
		ProjectModelSettings settings = ProjectModelSettings.current();
		List<Object> projects = new ArrayList<>();
		for (IProject project : ResourcesPlugin.getWorkspace().getRoot().getProjects()) {
			if (projects.size() >= MAX_PROJECTS || (monitor != null && monitor.isCanceled())) {
				break;
			}
			File location = ContainerResolver.location(project);
			IJavaProject javaProject = JavaCore.create(project);
			if (location == null || !javaProject.exists()) {
				continue;
			}
			List<Object> entries = new ArrayList<>();
			for (IClasspathEntry entry : javaProject.getRawClasspath()) {
				if (entries.size() >= MAX_ENTRIES_PER_PROJECT) {
					break;
				}
				Map<String, Object> described = describe(entry, javaProject, settings);
				if (described != null) {
					entries.add(described);
				}
			}
			Map<String, Object> item = new LinkedHashMap<>();
			item.put("name", project.getName());
			item.put("location", location.getPath());
			item.put("entries", entries);
			projects.add(item);
		}
		Map<String, Object> result = new LinkedHashMap<>();
		result.put("version", 1);
		result.put("enabled", settings.enabled);
		result.put("serverRuntimeName", settings.serverRuntimeName);
		result.put("projects", projects);
		return result;
	}

	private static Map<String, Object> describe(IClasspathEntry entry, IJavaProject project, ProjectModelSettings settings) {
		Map<String, Object> item = new LinkedHashMap<>();
		item.put("path", entry.getPath().toString());
		switch (entry.getEntryKind()) {
		case IClasspathEntry.CPE_CONTAINER -> {
			ContainerKind kind = ContainerKind.of(entry.getPath());
			item.put("kind", "container");
			item.put("containerKind", kind == null ? "other" : kind.wireName);
			IClasspathContainer container = null;
			try {
				container = JavaCore.getClasspathContainer(entry.getPath(), project);
			} catch (Exception e) {
				// Reported as unbound below; the cause is in JDT's own log.
			}
			if (kind != null && !settings.enabled) {
				item.put("status", "disabled");
			} else if (container == null) {
				item.put("status", "unbound");
			} else {
				int libraries = 0;
				int projects = 0;
				for (IClasspathEntry child : container.getClasspathEntries()) {
					if (child.getEntryKind() == IClasspathEntry.CPE_PROJECT) {
						projects++;
					} else {
						libraries++;
					}
				}
				item.put("status", libraries + projects == 0 ? "empty" : "resolved");
				item.put("libraries", libraries);
				item.put("projects", projects);
			}
			item.put("description", container != null ? container.getDescription()
					: kind != null ? ContainerResolver.description(entry.getPath(), kind, settings) : entry.getPath().toString());
			return item;
		}
		case IClasspathEntry.CPE_VARIABLE -> {
			item.put("kind", "variable");
			IClasspathEntry resolved = JavaCore.getResolvedClasspathEntry(entry);
			item.put("status", resolved == null ? "unbound" : resolved.getPath().toFile().exists() ? "resolved" : "missing");
			if (resolved != null) {
				item.put("resolvedPath", resolved.getPath().toOSString());
			}
			return item;
		}
		case IClasspathEntry.CPE_LIBRARY -> {
			IClasspathEntry resolved = JavaCore.getResolvedClasspathEntry(entry);
			if (resolved != null && libraryExists(resolved)) {
				return null;
			}
			item.put("kind", "library");
			item.put("status", "missing");
			return item;
		}
		default -> {
			return null;
		}
		}
	}

	/** A workspace-relative library is checked through its resource, others on disk. */
	private static boolean libraryExists(IClasspathEntry entry) {
		if (entry.getPath().toFile().exists()) {
			return true;
		}
		var resource = ResourcesPlugin.getWorkspace().getRoot().findMember(entry.getPath());
		return resource != null && resource.exists();
	}
}
