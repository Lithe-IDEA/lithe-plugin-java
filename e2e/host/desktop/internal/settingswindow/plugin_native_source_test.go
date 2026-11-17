//go:build vivahead && lithevisual

package settingswindow

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/1lck/Lithe-IDEA/desktop/internal/core"
	"github.com/1lck/Lithe-IDEA/desktop/internal/i18n"
	"github.com/1lck/Lithe-IDEA/desktop/internal/marketplace"
	"github.com/1lck/Lithe-IDEA/desktop/internal/pluginhost"
	"github.com/1lck/Lithe-IDEA/desktop/internal/pluginservice"
	"github.com/1lck/Lithe-IDEA/desktop/internal/pluginsettings"
	"github.com/vivaui/viva/headless"
)

// This plugin-owned overlay renders the public projection captured from its real
// engine tests. It does not publish files; native publication is tested separately.
type pluginNativeVisualSource struct {
	store                  *pluginsettings.Store
	manifest               json.RawMessage
	packageRoot, workspace string
	sources                pluginsettings.SourceEditor
	actionID               string
}

func (s *pluginNativeVisualSource) Load(ctx context.Context, _ string, language i18n.Language) (pluginservice.SettingsEditor, error) {
	snapshot, err := s.store.LoadPresented(ctx, s.manifest, s.workspace, pluginsettings.Presentation{Package: s.packageRoot, Language: language.Code()})
	return pluginservice.SettingsEditor{Snapshot: snapshot}, err
}
func (s *pluginNativeVisualSource) Save(context.Context, pluginservice.SettingsEditor, string, []core.PluginSettingEdit) (pluginsettings.SaveResult, error) {
	return pluginsettings.SaveResult{}, errors.New("visual capture never publishes")
}
func (s *pluginNativeVisualSource) Action(context.Context, pluginservice.SettingsEditor, string, bool) (pluginservice.SettingsActionResult, error) {
	return pluginservice.SettingsActionResult{Plan: core.PluginSettingsActionPlan{Kind: "invoke", ActionID: s.actionID, SourceEditor: true}, Sources: &s.sources}, nil
}
func (s *pluginNativeVisualSource) SaveSources(context.Context, pluginservice.SettingsEditor, string, pluginsettings.SourceEditor, []core.SourceSettingsEdits) (pluginsettings.SourceEditor, error) {
	return s.sources, errors.New("visual capture never publishes")
}

func TestPluginNativeSourceVisual(t *testing.T) {
	packageRoot, input, out := os.Getenv("LITHE_NATIVE_PACKAGE"), os.Getenv("LITHE_NATIVE_SNAPSHOTS"), os.Getenv("LITHE_NATIVE_OUTPUT")
	if packageRoot == "" || input == "" || out == "" {
		t.Fatal("run the plugin's scripts/compiler/visual.mjs")
	}
	manifest, err := os.ReadFile(filepath.Join(packageRoot, "plugin.json"))
	if err != nil {
		t.Fatal(err)
	}
	var metadata struct {
		ID, DisplayName, Version string
		SettingActions           []struct {
			ID   string
			Page struct{ CompilerID, LanguageID string }
		}
	}
	if err = json.Unmarshal(manifest, &metadata); err != nil {
		t.Fatal(err)
	}
	admission, cancelAdmission := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancelAdmission()
	if _, err = core.NewClient().ValidatePluginManifest(admission, manifest, pluginhost.Declared()); err != nil {
		t.Fatal(err)
	}
	compilerID, languageID := "", ""
	for _, action := range metadata.SettingActions {
		switch action.ID {
		case "compiler":
			compilerID = action.Page.CompilerID
		case "libraries":
			languageID = action.Page.LanguageID
		}
	}
	if compilerID == "" || languageID == "" {
		t.Fatal("compiler or libraries action missing")
	}
	// One view per captured projection: the compiler page, and the libraries
	// page of every project model scenario test-project-models.mjs captured.
	type view struct {
		name, actionID, key, snapshot string
		page                          pageID
		height                        int
	}
	views := []view{{name: "compiler", actionID: "compiler", key: "compiler:" + compilerID, snapshot: "visual-%s.json", page: pageBuild, height: 900}}
	scenarios, _ := filepath.Glob(filepath.Join(input, "visual-libraries-*-zh.json"))
	sort.Strings(scenarios)
	for _, path := range scenarios {
		scenario := strings.TrimSuffix(strings.TrimPrefix(filepath.Base(path), "visual-libraries-"), "-zh.json")
		views = append(views, view{name: "libraries-" + scenario, actionID: "libraries", key: "language:" + languageID, snapshot: "visual-libraries-" + scenario + "-%s.json", page: pageLanguages, height: 1600})
	}
	if err = os.MkdirAll(out, 0755); err != nil {
		t.Fatal(err)
	}
	for _, item := range views {
		for _, language := range []i18n.Language{i18n.Chinese, i18n.English} {
			for _, theme := range []string{"dark", "light"} {
				t.Run(item.name+"-"+language.Code()+"-"+theme, func(t *testing.T) {
					code := "en"
					if language == i18n.Chinese {
						code = "zh"
					}
					raw, err := os.ReadFile(filepath.Join(input, fmt.Sprintf(item.snapshot, code)))
					if err != nil {
						t.Fatal(err)
					}
					ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
					defer cancel()
					state, err := core.NewClient().ReadSourceSettings(ctx, raw)
					if err != nil {
						t.Fatal(err)
					}
					source := &pluginNativeVisualSource{store: pluginsettings.New(core.NewClient(), t.TempDir()), manifest: manifest, packageRoot: packageRoot, workspace: t.TempDir(), sources: pluginsettings.SourceEditor{State: state}, actionID: item.actionID}
					fetch := func(context.Context) (marketplace.Snapshot, error) {
						return marketplace.Snapshot{Index: core.MarketplaceIndex{Plugins: []core.MarketplacePlugin{{ID: metadata.ID, Name: metadata.DisplayName, LatestVersion: metadata.Version}}}}, nil
					}
					w := newWindow(NewController(Options{Text: func() *i18n.Messages { return i18n.For(language) }, Language: func() i18n.Language { return language }, SystemLanguage: language, SystemDark: func() bool { return theme == "dark" }, PluginSettings: source, SettingsNavigation: core.NewClient().SettingsNavigation, SettingsPages: func(ctx context.Context) ([]core.SettingsPage, error) {
						return core.NewClient().SettingsPages(ctx, []core.SettingsContribution{{Manifest: manifest, Owner: core.PluginSettingsOwner{PluginID: metadata.ID, Version: metadata.Version}, Enabled: true}})
					}, Marketplace: fetch, MarketplaceInstalled: fetch, PluginState: func(string) PluginState {
						return PluginState{Installed: true, Enabled: true, Version: metadata.Version}
					}}), Values{Theme: theme, Language: language.Code()})
					config := w.config()
					config.Width, config.Height = 1120, item.height
					session, err := headless.Open(config, nil, headless.Options{})
					if err != nil {
						t.Fatal(err)
					}
					t.Cleanup(session.Close)
					d := &windowDriver{t: t, w: w, driver: session.Driver()}
					d.until("plugin contribution", func() bool { return len(w.dynamic.pages) > 0 })
					w.selectSettingsPage(item.page, item.key)
					d.until("read action", func() bool { return w.pluginSettings.pageKey == w.dynamicPage && w.pluginSettings.loaded })
					title := ""
					for _, action := range w.pluginSettings.editor.Snapshot.State.Actions {
						if action.ID == item.actionID {
							title = action.Title
						}
					}
					if title == "" {
						t.Fatalf("action %s is not on page %s", item.actionID, item.key)
					}
					if item.actionID == "libraries" && item.name == views[1].name {
						// The language page itself, with the plugin's own settings.
						d.pump()
						if err = session.Screenshot(filepath.Join(out, "java-settings-"+language.Code()+"-"+theme+".png")); err != nil {
							t.Fatal(err)
						}
					}
					d.click(title)
					d.until("native source form", func() bool { return w.pluginSettings.sources != nil })
					d.pump()
					if err = session.Screenshot(filepath.Join(out, item.name+"-"+language.Code()+"-"+theme+".png")); err != nil {
						t.Fatal(err)
					}
				})
			}
		}
	}
}
