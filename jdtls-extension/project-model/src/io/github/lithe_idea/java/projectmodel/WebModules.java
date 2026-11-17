package io.github.lithe_idea.java.projectmodel;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.jar.Manifest;

import javax.xml.XMLConstants;
import javax.xml.parsers.DocumentBuilder;
import javax.xml.parsers.DocumentBuilderFactory;

import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.NodeList;

/**
 * Static reading of the module layout Eclipse WTP and MyEclipse keep in project
 * files. Only fixed paths are read, every read is bounded, and a file that is
 * missing or malformed contributes nothing: the container then resolves to
 * fewer libraries instead of failing the project.
 */
final class WebModules {
	/** Upper bound of libraries one directory contributes. */
	static final int MAX_LIBRARIES_PER_DIRECTORY = 4096;
	/** Descriptor files larger than this are not module descriptors. */
	static final long MAX_DESCRIPTOR_BYTES = 1024 * 1024;
	static final String COMPONENT = ".settings/org.eclipse.wst.common.component";
	static final String MYECLIPSE_METADATA = ".mymetadata";
	/** Web roots of common tools, tried in order when no descriptor names one. */
	static final List<String> DEFAULT_WEB_ROOTS = List.of("WebContent", "WebRoot", "src/main/webapp", "web", "webapp", "WebApp");
	/** Content roots of EJB, connector and utility modules without a descriptor. */
	static final List<String> DEFAULT_MODULE_ROOTS = List.of("ejbModule", "connectorModule", "appClientModule", "src");
	static final String DEFAULT_EAR_ROOT = "EarContent";

	private WebModules() {
	}

	/** A {@code <dependent-module>} of a WTP component. */
	record DependentModule(String handle, String deployPath, String archiveName) {
		/** The project of a {@code module:/resource/<project>/<module>} handle, or null. */
		String projectName() {
			String prefix = "module:/resource/";
			if (handle == null || !handle.startsWith(prefix)) {
				return null;
			}
			String rest = handle.substring(prefix.length());
			int slash = rest.indexOf('/');
			return slash > 0 ? rest.substring(0, slash) : null;
		}

		/** The path of a {@code module:/classpath/lib/<path>} handle, or null. */
		String libraryPath() {
			String prefix = "module:/classpath/lib/";
			return handle != null && handle.startsWith(prefix) ? handle.substring(prefix.length()) : null;
		}

		/** {@code [variable, rest]} of a {@code module:/classpath/var/<variable>/<rest>} handle, or null. */
		String[] variablePath() {
			String prefix = "module:/classpath/var/";
			if (handle == null || !handle.startsWith(prefix)) {
				return null;
			}
			String rest = handle.substring(prefix.length());
			int slash = rest.indexOf('/');
			return slash > 0 ? new String[] { rest.substring(0, slash), rest.substring(slash + 1) } : null;
		}

		/** Whether the module is deployed into {@code directory} ("/WEB-INF/lib", "/lib"). */
		boolean deployedTo(String directory) {
			return deployPath != null && trimSlashes(deployPath).equals(trimSlashes(directory));
		}
	}

	/** The parts of a WTP component file this bundle uses. */
	record Component(List<String> rootSources, List<DependentModule> dependents) {
	}

	/** The WTP component of the project at {@code projectDirectory}, or null. */
	static Component readComponent(File projectDirectory) {
		Element root = parse(new File(projectDirectory, COMPONENT));
		if (root == null) {
			return null;
		}
		List<String> sources = new ArrayList<>();
		List<DependentModule> dependents = new ArrayList<>();
		NodeList modules = root.getElementsByTagName("wb-module");
		for (int index = 0; index < modules.getLength(); index++) {
			Element module = (Element) modules.item(index);
			NodeList resources = module.getElementsByTagName("wb-resource");
			for (int item = 0; item < resources.getLength(); item++) {
				Element resource = (Element) resources.item(item);
				if ("/".equals(resource.getAttribute("deploy-path"))) {
					String source = trimSlashes(resource.getAttribute("source-path"));
					if (!source.isEmpty()) {
						sources.add(source);
					}
				}
			}
			NodeList children = module.getElementsByTagName("dependent-module");
			for (int item = 0; item < children.getLength(); item++) {
				Element child = (Element) children.item(item);
				dependents.add(new DependentModule(child.getAttribute("handle"), child.getAttribute("deploy-path"), child.getAttribute("archiveName")));
			}
		}
		return new Component(List.copyOf(sources), List.copyOf(dependents));
	}

	/**
	 * The directory deployed as the web application root: the WTP component's
	 * root resource that holds WEB-INF, else MyEclipse's {@code webrootdir},
	 * else the first of {@link #DEFAULT_WEB_ROOTS} that holds WEB-INF.
	 */
	static File webRoot(File projectDirectory) {
		List<String> candidates = new ArrayList<>();
		Component component = readComponent(projectDirectory);
		if (component != null) {
			candidates.addAll(component.rootSources());
		}
		Element metadata = parse(new File(projectDirectory, MYECLIPSE_METADATA));
		if (metadata != null) {
			NodeList modules = metadata.getElementsByTagName("project-module");
			for (int index = 0; index < modules.getLength(); index++) {
				String root = trimSlashes(((Element) modules.item(index)).getAttribute("webrootdir"));
				if (!root.isEmpty()) {
					candidates.add(root);
				}
			}
		}
		candidates.addAll(DEFAULT_WEB_ROOTS);
		for (String candidate : candidates) {
			File directory = new File(projectDirectory, candidate);
			if (new File(directory, "WEB-INF").isDirectory()) {
				return directory;
			}
		}
		return null;
	}

	/** The directory whose META-INF/MANIFEST.MF describes the module, or null. */
	static File moduleRoot(File projectDirectory) {
		List<String> candidates = new ArrayList<>();
		Component component = readComponent(projectDirectory);
		if (component != null) {
			candidates.addAll(component.rootSources());
		}
		File web = webRoot(projectDirectory);
		if (web != null) {
			candidates.add(0, projectDirectory.toPath().relativize(web.toPath()).toString());
		}
		candidates.addAll(DEFAULT_MODULE_ROOTS);
		for (String candidate : candidates) {
			File directory = new File(projectDirectory, candidate);
			if (new File(directory, "META-INF/MANIFEST.MF").isFile()) {
				return directory;
			}
		}
		return null;
	}

	/** The EAR content root of an EAR project with {@code component}. */
	static File earRoot(File projectDirectory, Component component) {
		for (String source : component.rootSources()) {
			File directory = new File(projectDirectory, source);
			if (directory.isDirectory()) {
				return directory;
			}
		}
		return new File(projectDirectory, DEFAULT_EAR_ROOT);
	}

	/**
	 * The library directory of the EAR at {@code earRoot}: application.xml's
	 * {@code <library-directory>}, else the Java EE 5 default "lib".
	 */
	static String earLibraryDirectory(File earRoot) {
		Element application = parse(new File(earRoot, "META-INF/application.xml"));
		if (application != null) {
			NodeList directories = application.getElementsByTagNameNS("*", "library-directory");
			if (directories.getLength() == 0) {
				directories = application.getElementsByTagName("library-directory");
			}
			if (directories.getLength() > 0) {
				String directory = trimSlashes(directories.item(0).getTextContent().trim());
				if (!directory.isEmpty() && !directory.contains("..")) {
					return directory;
				}
			}
		}
		return "lib";
	}

	/** The relative entries of the manifest Class-Path of the module at {@code moduleRoot}. */
	static List<String> manifestClassPath(File moduleRoot) {
		if (moduleRoot == null) {
			return List.of();
		}
		File file = new File(moduleRoot, "META-INF/MANIFEST.MF");
		if (!file.isFile() || file.length() > MAX_DESCRIPTOR_BYTES) {
			return List.of();
		}
		try (InputStream input = new FileInputStream(file)) {
			String value = new Manifest(input).getMainAttributes().getValue("Class-Path");
			if (value == null) {
				return List.of();
			}
			List<String> entries = new ArrayList<>();
			for (String entry : value.trim().split("\\s+")) {
				if (!entry.isEmpty() && !entry.contains("..") && !entry.contains(":")) {
					entries.add(entry);
				}
			}
			return entries;
		} catch (IOException | RuntimeException e) {
			// A malformed manifest only means no Class-Path entries.
			return List.of();
		}
	}

	/** The JAR and ZIP files directly inside {@code directory}, in name order. */
	static List<File> libraries(File directory) {
		File[] files = directory == null ? null : directory.listFiles();
		if (files == null) {
			return List.of();
		}
		Arrays.sort(files);
		List<File> libraries = new ArrayList<>();
		for (File file : files) {
			String name = file.getName().toLowerCase(java.util.Locale.ROOT);
			if (file.isFile() && (name.endsWith(".jar") || name.endsWith(".zip")) && !name.endsWith("-sources.jar")) {
				libraries.add(file);
				if (libraries.size() >= MAX_LIBRARIES_PER_DIRECTORY) {
					break;
				}
			}
		}
		return libraries;
	}

	/** {@code foo-sources.jar} next to {@code foo.jar}, when present. */
	static File sourceAttachment(File library) {
		String name = library.getName();
		int dot = name.lastIndexOf('.');
		if (dot <= 0) {
			return null;
		}
		File candidate = new File(library.getParentFile(), name.substring(0, dot) + "-sources.jar");
		return candidate.isFile() ? candidate : null;
	}

	static String trimSlashes(String value) {
		String trimmed = value == null ? "" : value.trim().replace('\\', '/');
		while (trimmed.startsWith("/")) {
			trimmed = trimmed.substring(1);
		}
		while (trimmed.endsWith("/")) {
			trimmed = trimmed.substring(0, trimmed.length() - 1);
		}
		return trimmed;
	}

	/** The document element of {@code file}, with DTDs and external entities refused. */
	private static Element parse(File file) {
		if (!file.isFile() || file.length() > MAX_DESCRIPTOR_BYTES) {
			return null;
		}
		try {
			DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
			factory.setNamespaceAware(true);
			factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", false);
			factory.setFeature("http://apache.org/xml/features/nonvalidating/load-external-dtd", false);
			factory.setFeature("http://xml.org/sax/features/external-general-entities", false);
			factory.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
			factory.setAttribute(XMLConstants.ACCESS_EXTERNAL_DTD, "");
			factory.setAttribute(XMLConstants.ACCESS_EXTERNAL_SCHEMA, "");
			factory.setExpandEntityReferences(false);
			DocumentBuilder builder = factory.newDocumentBuilder();
			builder.setErrorHandler(null);
			builder.setEntityResolver((publicId, systemId) -> new org.xml.sax.InputSource(new java.io.StringReader("")));
			Document document = builder.parse(file);
			return document.getDocumentElement();
		} catch (Exception e) {
			// Malformed descriptors are skipped; see the class comment.
			return null;
		}
	}
}
