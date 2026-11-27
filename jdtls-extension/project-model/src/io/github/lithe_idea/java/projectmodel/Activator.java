package io.github.lithe_idea.java.projectmodel;

import org.eclipse.core.resources.IResourceChangeEvent;
import org.eclipse.core.resources.ResourcesPlugin;
import org.eclipse.jdt.ls.core.internal.JavaLanguageServerPlugin;
import org.eclipse.jdt.ls.core.internal.preferences.PreferenceManager;
import org.osgi.framework.BundleActivator;
import org.osgi.framework.BundleContext;

/** Owns the listeners of the bundle; both are removed when JDT LS stops it. */
public final class Activator implements BundleActivator {
	private ContainerRefresher refresher;

	@Override
	public void start(BundleContext context) {
		refresher = new ContainerRefresher();
		PreferenceManager preferences = JavaLanguageServerPlugin.getPreferencesManager();
		if (preferences != null) {
			preferences.addPreferencesChangeListener(refresher);
		}
		ResourcesPlugin.getWorkspace().addResourceChangeListener(refresher, IResourceChangeEvent.POST_CHANGE);
	}

	@Override
	public void stop(BundleContext context) {
		if (refresher == null) {
			return;
		}
		PreferenceManager preferences = JavaLanguageServerPlugin.getPreferencesManager();
		if (preferences != null) {
			preferences.removePreferencesChangeListener(refresher);
		}
		ResourcesPlugin.getWorkspace().removeResourceChangeListener(refresher);
		refresher.cancel();
		refresher = null;
	}
}
