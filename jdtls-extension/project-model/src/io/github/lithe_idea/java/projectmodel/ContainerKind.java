package io.github.lithe_idea.java.projectmodel;

import java.util.Map;

import org.eclipse.core.runtime.IPath;

/**
 * The classpath containers this bundle resolves, keyed by the first segment of
 * the container path the way JDT selects an initializer. Must match plugin.xml.
 */
enum ContainerKind {
	/** WTP "Web App Libraries": the module's WEB-INF/lib and utility modules. */
	WEB_APP("webApp"),
	/** WTP "EAR Libraries": the lib directory of every EAR that bundles the module. */
	EAR("ear"),
	/** WTP "Server Runtime": the servlet container's own libraries. */
	SERVER_RUNTIME("serverRuntime"),
	/** MyEclipse Java EE API libraries; served by the servlet container too. */
	JAVA_EE("javaEE");

	private static final Map<String, ContainerKind> BY_ID = Map.of(
			"org.eclipse.jst.j2ee.internal.web.container", WEB_APP,
			"org.eclipse.jst.j2ee.internal.module.container", EAR,
			"org.eclipse.jst.server.core.container", SERVER_RUNTIME,
			"com.genuitec.eclipse.j2eedt.core.J2EE13_CONTAINER", JAVA_EE,
			"com.genuitec.eclipse.j2eedt.core.J2EE14_CONTAINER", JAVA_EE,
			"melibrary.com.genuitec.eclipse.j2eedt.core.MYECLIPSE_JAVAEE_5_CONTAINER", JAVA_EE,
			"melibrary.com.genuitec.eclipse.j2eedt.core.MYECLIPSE_JAVAEE_6_CONTAINER", JAVA_EE,
			"melibrary.com.genuitec.eclipse.j2eedt.core.MYECLIPSE_JAVAEE_7_CONTAINER", JAVA_EE,
			"com.genuitec.runtime.library", JAVA_EE);

	/** Stable identifier reported to the plugin backend. */
	final String wireName;

	ContainerKind(String wireName) {
		this.wireName = wireName;
	}

	/** The kind of {@code containerPath}, or null when another initializer owns it. */
	static ContainerKind of(IPath containerPath) {
		if (containerPath == null || containerPath.segmentCount() == 0) {
			return null;
		}
		return BY_ID.get(containerPath.segment(0));
	}

	/** Whether the container needs the servlet container the backend configures. */
	boolean usesServerRuntime() {
		return this == SERVER_RUNTIME || this == JAVA_EE;
	}
}
