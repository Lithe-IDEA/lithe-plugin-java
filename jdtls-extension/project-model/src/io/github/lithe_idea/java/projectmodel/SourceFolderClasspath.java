package io.github.lithe_idea.java.projectmodel;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.eclipse.core.resources.IResource;
import org.eclipse.core.resources.IFolder;
import org.eclipse.core.runtime.CoreException;
import org.eclipse.core.runtime.IPath;
import org.eclipse.core.runtime.IProgressMonitor;
import org.eclipse.core.runtime.Path;
import org.eclipse.jdt.core.IClasspathAttribute;
import org.eclipse.jdt.core.IClasspathEntry;
import org.eclipse.jdt.core.IJavaProject;
import org.eclipse.jdt.core.JavaCore;
import org.eclipse.jdt.ls.core.internal.ProjectUtils;
import org.eclipse.jdt.ls.core.internal.managers.InvisibleProjectImporter;

/** Applies workspace folder roles to JDT's unmanaged project metadata. */
final class SourceFolderClasspath {
	private static final String TEST_OWNER = "lithe.testSource";
	private static final String TEST_OUTPUT_OWNER = "lithe.originalTestOutput";
	private static final String EXCLUSION_OWNER = "lithe.sourceExclusions";

	static void refresh(ProjectModelSettings settings, IProgressMonitor monitor) {
		for (IJavaProject project : ProjectUtils.getJavaProjects()) {
			if (monitor.isCanceled()) {
				return;
			}
			if (!ProjectUtils.isUnmanagedFolder(project.getProject())) {
				continue;
			}
			IPath workspace = ProjectUtils.getProjectRealFolder(project.getProject());
			boolean matches = ProjectModelSettings.sameDirectory(workspace, settings.classFolderWorkspace);
			Map<String, List<String>> roles = matches ? settings.sourceRoles : Map.of();
			try {
				IClasspathEntry[] before = project.getRawClasspath();
				IPath oldOutput = project.getOutputLocation();
				IPath output = matches && settings.workspaceOutputPath.isPresent()
						? InvisibleProjectImporter.getOutputPath(project, settings.workspaceOutputPath.get(), true) : oldOutput;
				IClasspathEntry[] after = matches && (settings.workspaceSourcePaths.isPresent() || !output.equals(oldOutput))
						? resolveSources(project, before, settings.workspaceSourcePaths.orElse(null), output, oldOutput)
						: before.clone();
				IPath testOutput = project.getProject().getFullPath().append("lithe-test-bin");
				for (int index = 0; index < after.length; index++) {
					IClasspathEntry entry = after[index];
					if (entry.getEntryKind() == IClasspathEntry.CPE_SOURCE) {
						IResource resource = entry.getPath().segmentCount() == 1 ? project.getProject()
								: project.getProject().getWorkspace().getRoot().getFolder(entry.getPath());
						after[index] = apply(entry, resource.getLocation(), roles, output, testOutput);
					}
				}
				if (!Arrays.equals(before, after) || !output.equals(oldOutput)) {
					project.setRawClasspath(after, output, monitor);
				}
			} catch (CoreException e) {
				ProjectModelLog.error("Cannot update Java source folder roles for " + project.getElementName(), e);
			}
		}
	}

	private static IClasspathEntry[] resolveSources(IJavaProject project, IClasspathEntry[] before, List<String> paths,
			IPath output, IPath oldOutput)
			throws CoreException {
		IFolder link = project.getProject().getFolder(ProjectUtils.WORKSPACE_LINK);
		if (!link.exists()) {
			return before.clone();
		}
		for (String path : paths == null ? List.<String>of() : paths) {
			IPath relative = Path.fromPortableString(path);
			if (!relative.isEmpty() && relative.segmentCount() > 0 && !relative.isAbsolute() && relative.getDevice() == null
					&& !Arrays.asList(relative.segments()).contains("..")) {
				IFolder folder = link.getFolder(relative);
				if (!folder.exists()) {
					folder.refreshLocal(IResource.DEPTH_INFINITE, null);
				}
			}
		}
		List<IPath> existing = Arrays.stream(before).filter(entry -> entry.getEntryKind() == IClasspathEntry.CPE_SOURCE)
				.map(IClasspathEntry::getPath).toList();
		List<IPath> desired = paths == null ? existing : InvisibleProjectImporter.getSourcePaths(paths, link);
		if (output.equals(oldOutput) && new LinkedHashSet<>(existing).equals(new LinkedHashSet<>(desired))) {
			return before.clone();
		}
		Map<IPath, IPath> sources = new LinkedHashMap<>();
		for (IPath path : desired) {
			sources.put(path, null);
		}
		List<IPath> excludes = InvisibleProjectImporter.getExcludingPath(project, output, link);
		return ProjectUtils.resolveClassPathEntries(project, sources, excludes, output);
	}

	private static IClasspathEntry apply(IClasspathEntry entry, IPath folder, Map<String, List<String>> roles,
			IPath defaultOutput, IPath testOutput) {
		List<IClasspathAttribute> attributes = new ArrayList<>(Arrays.asList(entry.getExtraAttributes()));
		Set<IPath> exclusions = new LinkedHashSet<>(Arrays.asList(entry.getExclusionPatterns()));
		IPath output = entry.getOutputLocation();
		boolean ownedTest = attributes.removeIf(attribute -> TEST_OWNER.equals(attribute.getName()));
		if (ownedTest) {
			attributes.removeIf(attribute -> IClasspathAttribute.TEST.equals(attribute.getName()));
		}
		for (IClasspathAttribute attribute : List.copyOf(attributes)) {
			if (EXCLUSION_OWNER.equals(attribute.getName())) {
				for (String pattern : attribute.getValue().split("\\|")) {
					exclusions.remove(Path.fromPortableString(pattern));
				}
				attributes.remove(attribute);
			} else if (TEST_OUTPUT_OWNER.equals(attribute.getName())) {
				output = attribute.getValue().isEmpty() ? null : Path.fromPortableString(attribute.getValue());
				attributes.remove(attribute);
			}
		}
		java.nio.file.Path folderPath = folder == null ? null : ProjectModelSettings.realPath(folder.toFile().toPath());
		if (folderPath != null && !roles.isEmpty()) {
			boolean testResource = contains(roles, "test-resource", folderPath);
			boolean test = testResource || contains(roles, "test", folderPath);
			if (test
					&& attributes.stream().noneMatch(attribute -> IClasspathAttribute.TEST.equals(attribute.getName()))) {
				attributes.add(JavaCore.newClasspathAttribute(IClasspathAttribute.TEST, "true"));
				attributes.add(JavaCore.newClasspathAttribute(TEST_OWNER, "true"));
			}
			if (test && (output == null || output.equals(defaultOutput))) {
				attributes.add(JavaCore.newClasspathAttribute(TEST_OUTPUT_OWNER, output == null ? "" : output.toPortableString()));
				output = testOutput;
			}
			List<String> added = new ArrayList<>();
			if (testResource || contains(roles, "resource", folderPath)) {
				addExclusion(exclusions, added, "**/*.java");
			}
			for (String path : roles.getOrDefault("excluded", List.of())) {
				java.nio.file.Path excluded = ProjectModelSettings.realPath(path);
				if (excluded != null && excluded.startsWith(folderPath) && !excluded.equals(folderPath)) {
					java.nio.file.Path relative = folderPath.relativize(excluded);
					StringBuilder portable = new StringBuilder();
					for (int i = 0; i < relative.getNameCount(); i++) {
						if (i > 0) {
							portable.append('/');
						}
						portable.append(relative.getName(i).toString());
					}
					if (portable.length() > 0) {
						addExclusion(exclusions, added, portable.append('/').toString());
					}
				}
			}
			if (!added.isEmpty()) {
				attributes.add(JavaCore.newClasspathAttribute(EXCLUSION_OWNER, String.join("|", added)));
			}
		}
		if (Arrays.equals(entry.getExtraAttributes(), attributes.toArray(IClasspathAttribute[]::new))
				&& Arrays.equals(entry.getExclusionPatterns(), exclusions.toArray(IPath[]::new))
				&& java.util.Objects.equals(entry.getOutputLocation(), output)) {
			return entry;
		}
		return JavaCore.newSourceEntry(entry.getPath(), entry.getInclusionPatterns(),
				exclusions.toArray(IPath[]::new), output, attributes.toArray(IClasspathAttribute[]::new));
	}

	private static boolean contains(Map<String, List<String>> roles, String role, java.nio.file.Path folderPath) {
		return roles.getOrDefault(role, List.of()).stream()
				.map(ProjectModelSettings::realPath)
				.anyMatch(folderPath::equals);
	}

	private static void addExclusion(Set<IPath> exclusions, List<String> added, String pattern) {
		if (exclusions.add(Path.fromPortableString(pattern))) {
			added.add(pattern);
		}
	}
}
