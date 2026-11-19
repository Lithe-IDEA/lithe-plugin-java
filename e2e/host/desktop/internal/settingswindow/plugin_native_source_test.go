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
	"sync"
	"testing"
	"time"

	"github.com/1lck/Lithe-IDEA/desktop/internal/core"
	"github.com/1lck/Lithe-IDEA/desktop/internal/i18n"
	"github.com/1lck/Lithe-IDEA/desktop/internal/marketplace"
	"github.com/1lck/Lithe-IDEA/desktop/internal/pluginhost"
	"github.com/1lck/Lithe-IDEA/desktop/internal/pluginservice"
	"github.com/1lck/Lithe-IDEA/desktop/internal/pluginsettings"
	"github.com/vivaui/viva/gfx"
	"github.com/vivaui/viva/headless"
)

type visualBackend struct {
	client    *core.Client
	sessionID string
	pluginID  string
	mu        sync.Mutex
	wg        sync.WaitGroup
}

func startVisualBackend(t *testing.T, client *core.Client, pluginID, executablePath, workDir string) *visualBackend {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	started, err := client.StartPluginBackend(ctx, core.PluginBackendStart{
		ProviderID:                    "visual-backend",
		PluginID:                      pluginID,
		ExecutablePath:                executablePath,
		RootURI:                       core.NativeDocumentPath{Path: workDir, Platform: "auto"},
		WorkingDirectory:              workDir,
		InitializeTimeoutMilliseconds: 5000,
		RequestTimeoutMilliseconds:    5000,
		ShutdownTimeoutMilliseconds:   3000,
	})
	if err != nil {
		t.Fatal(err)
	}
	backend := &visualBackend{client: client, sessionID: started.SessionID, pluginID: pluginID}
	t.Cleanup(func() {
		backend.wg.Wait()
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 8*time.Second)
		defer cleanupCancel()
		if stopErr := client.StopPluginBackend(cleanupCtx, started.SessionID); stopErr != nil {
			t.Errorf("stop visual backend: %v", stopErr)
		}
		for {
			destroyErr := client.DestroyPluginBackend(cleanupCtx, started.SessionID)
			if destroyErr == nil {
				return
			}
			var coreErr *core.Error
			if !errors.As(destroyErr, &coreErr) || coreErr.Code != core.CodeInvalidRequest || coreErr.Message != "A running language-server session cannot be destroyed." {
				t.Errorf("destroy visual backend: %v", destroyErr)
				return
			}
			if _, waitErr := client.WaitBackendEvents(cleanupCtx, started.SessionID, 250); waitErr != nil {
				t.Errorf("wait visual backend shutdown: %v", waitErr)
				return
			}
		}
	})
	registered, ready := false, false
	for !(registered && ready) {
		events, waitErr := client.WaitBackendEvents(ctx, started.SessionID, 1000)
		if waitErr != nil {
			t.Fatal(waitErr)
		}
		for _, event := range events {
			if event.Type == "functionsChanged" && strings.Contains(string(event.Result), `"settings.view"`) {
				registered = true
			}
			if event.Type == "stateChanged" && event.State == "ready" {
				ready = true
			}
		}
	}
	return backend
}

func (b *visualBackend) invoke(ctx context.Context, function string, params json.RawMessage) (json.RawMessage, error) {
	b.wg.Add(1)
	defer b.wg.Done()
	b.mu.Lock()
	defer b.mu.Unlock()
	callCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	op, err := b.client.InvokePluginFunction(callCtx, b.sessionID, "", b.pluginID, function, params)
	if err != nil {
		return nil, err
	}
	for {
		events, waitErr := b.client.WaitBackendEvents(callCtx, b.sessionID, 1000)
		if waitErr != nil {
			return nil, waitErr
		}
		for _, event := range events {
			if event.Type == "requestCompleted" && event.OperationID == op {
				if len(event.Error) != 0 && string(event.Error) != "null" {
					return nil, fmt.Errorf("backend function %s failed: %s", function, event.Error)
				}
				var wrapped struct {
					Value json.RawMessage `json:"value"`
				}
				if err := json.Unmarshal(event.Result, &wrapped); err != nil {
					return nil, fmt.Errorf("decode function result %s: %w", function, err)
				}
				if len(wrapped.Value) == 0 {
					return json.RawMessage("null"), nil
				}
				return wrapped.Value, nil
			}
		}
	}
}

// This plugin-owned overlay renders the public projection captured from its real
// engine tests. It does not publish files; native publication is tested separately.
type pluginNativeVisualSource struct {
	store                  *pluginsettings.Store
	manifest               json.RawMessage
	packageRoot, workspace string
	sources                pluginsettings.SourceEditor
	actionID               string
	backend                *visualBackend
}

func (s *pluginNativeVisualSource) Load(ctx context.Context, _ string, language i18n.Language) (pluginservice.SettingsEditor, error) {
	snapshot, err := s.store.LoadPresented(ctx, s.manifest, s.workspace, pluginsettings.Presentation{Package: s.packageRoot, Language: language.Code()})
	return pluginservice.SettingsEditor{Snapshot: snapshot}, err
}
func (s *pluginNativeVisualSource) Save(context.Context, pluginservice.SettingsEditor, string, []core.PluginSettingEdit) (pluginsettings.SaveResult, error) {
	return pluginsettings.SaveResult{}, errors.New("visual capture never publishes")
}
func (s *pluginNativeVisualSource) Action(ctx context.Context, editor pluginservice.SettingsEditor, id string, confirmed bool) (pluginservice.SettingsActionResult, error) {
	return pluginservice.SettingsActionResult{Plan: core.PluginSettingsActionPlan{Kind: "invoke", ActionID: s.actionID, SourceEditor: true}, Sources: &s.sources}, nil
}
func (s *pluginNativeVisualSource) SaveSources(context.Context, pluginservice.SettingsEditor, string, pluginsettings.SourceEditor, []core.SourceSettingsEdits) (pluginsettings.SourceEditor, error) {
	return s.sources, errors.New("visual capture never publishes")
}
func (s *pluginNativeVisualSource) View(ctx context.Context, _ pluginservice.SettingsEditor, region, function string, event, view json.RawMessage, closing bool) (json.RawMessage, bool, error) {
	params := map[string]json.RawMessage{}
	if len(event) > 0 {
		if err := json.Unmarshal(event, &params); err != nil {
			return nil, false, err
		}
	}
	surface, err := json.Marshal(core.PluginViewSurface{SettingRegionID: region})
	if err != nil {
		return nil, false, err
	}
	params["surface"] = surface
	payload, err := json.Marshal(params)
	if err != nil {
		return nil, false, err
	}
	answer, err := s.backend.invoke(ctx, function, payload)
	if err != nil || closing {
		return nil, false, err
	}
	if len(answer) == 0 || string(answer) == "null" {
		answer = json.RawMessage(`{}`)
	}
	return s.backend.client.ApplyPluginViewResult(ctx, s.manifest, core.PluginViewSurface{SettingRegionID: region}, view, answer)
}

func TestPluginNativeSourceVisual(t *testing.T) {
	packageRoot, backendPath, input, out := os.Getenv("LITHE_NATIVE_PACKAGE"), os.Getenv("LITHE_NATIVE_BACKEND"), os.Getenv("LITHE_NATIVE_SNAPSHOTS"), os.Getenv("LITHE_NATIVE_OUTPUT")
	if packageRoot == "" || backendPath == "" || input == "" || out == "" {
		t.Fatal("run the plugin's scripts/compiler/visual.mjs")
	}
	manifest, err := os.ReadFile(filepath.Join(packageRoot, "plugin.json"))
	if err != nil {
		t.Fatal(err)
	}
	zhLocalization, err := os.ReadFile(filepath.Join(packageRoot, "l10n", "zh-Hans.json"))
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
	coreClient := core.NewClient()
	admission, cancelAdmission := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancelAdmission()
	if _, err = coreClient.ValidatePluginManifest(admission, manifest, pluginhost.Declared()); err != nil {
		t.Fatal(err)
	}
	backend := startVisualBackend(t, coreClient, metadata.ID, backendPath, t.TempDir())
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
					source := &pluginNativeVisualSource{store: pluginsettings.New(coreClient, t.TempDir()), manifest: manifest, packageRoot: packageRoot, workspace: t.TempDir(), sources: pluginsettings.SourceEditor{State: state}, actionID: item.actionID, backend: backend}
					fetch := func(context.Context) (marketplace.Snapshot, error) {
						return marketplace.Snapshot{Index: core.MarketplaceIndex{Plugins: []core.MarketplacePlugin{{ID: metadata.ID, Name: metadata.DisplayName, LatestVersion: metadata.Version}}}}, nil
					}
					w := newWindow(NewController(Options{SettingsError: func(operation string, err error) {
						t.Logf("settings error %s: %v", operation, err)
					}, Text: func() *i18n.Messages { return i18n.For(language) }, Language: func() i18n.Language { return language }, SystemLanguage: language, SystemDark: func() bool { return theme == "dark" }, PluginSettings: source, SettingsNavigation: coreClient.SettingsNavigation, SettingsPages: func(ctx context.Context, lang i18n.Language) ([]core.SettingsPage, error) {
						localizations := map[string]string{}
						if lang == i18n.Chinese {
							localizations["l10n/zh-Hans.json"] = string(zhLocalization)
						}
						pages, pagesErr := coreClient.SettingsPages(ctx, []core.SettingsContribution{{Manifest: manifest, Owner: core.PluginSettingsOwner{PluginID: metadata.ID, Version: metadata.Version}, Enabled: true, Localizations: localizations}}, lang.Code())
						if pagesErr != nil {
							t.Logf("SettingsPages(%s) error: %v", lang.Code(), pagesErr)
						}
						return pages, pagesErr
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
					d.until("region preview", func() bool {
						if w.region != nil && w.region.session.Err() != nil {
							t.Fatalf("region session err: %v", w.region.session.Err())
						}
						return w.region != nil && w.region.session.Drawn()
					})
					if item.actionID == "compiler" || (item.actionID == "libraries" && item.name == views[1].name) {
						d.pump()
						prefix := "java-settings-"
						if item.actionID == "compiler" {
							prefix = "java-compiler-settings-"
						}
						if err = session.Screenshot(filepath.Join(out, prefix+language.Code()+"-"+theme+".png")); err != nil {
							t.Fatal(err)
						}
					}
					d.click(title)
					d.until("native source form", func() bool { return w.pluginSettings.sources != nil })
					shot := func(name string) {
						d.pump()
						if err = session.Screenshot(filepath.Join(out, name+"-"+language.Code()+"-"+theme+".png")); err != nil {
							t.Fatal(err)
						}
					}
					shot(item.name)
					if item.name == "libraries-modules" {
						// The Eclipse module's dependencies tab, then the Maven
						// module JDT resolved, read-only.
						dependencies := "Dependencies"
						if language == i18n.Chinese {
							dependencies = "依赖"
						}
						d.click(dependencies)
						shot(item.name + "-dependencies")
						d.click("service")
						shot(item.name + "-maven")
						// The Web Facet source of app: the lower of the two
						// sidebar entries titled app, under the Web Facet group.
						var facet gfx.Hit
						found := false
						for _, hit := range d.driver.Snapshot().Hits {
							if hit.Label == "app" && !hit.IsLabel && !hit.Backdrop && !hit.ID.IsZero() && (!found || hit.Bounds.Y > facet.Bounds.Y) {
								facet, found = hit, true
							}
						}
						if !found {
							t.Fatal("no Web Facet source titled app")
						}
						d.clickHit(facet)
						shot(item.name + "-web-facet")
					}
				})
			}
		}
	}
}
