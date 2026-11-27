// This integration fixture belongs to the Java plugin, not the generic host.
// test-host-managed.mjs overlays it into a matching host checkout for execution.
// It uses the real Core contract, SDK backend, host service and runtime manager.
// Only HTTPS transport is replaced with previously verified official archives.
package pluginservice_test

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/1lck/Lithe-IDEA/desktop/internal/core"
	"github.com/1lck/Lithe-IDEA/desktop/internal/marketplace"
	"github.com/1lck/Lithe-IDEA/desktop/internal/pluginservice"
	"github.com/1lck/Lithe-IDEA/desktop/internal/store"
)

type javaArchiveTransport func(*http.Request) (*http.Response, error)

func (f javaArchiveTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	return f(request)
}

func TestJavaManagedRuntimeOfficialArchives(t *testing.T) {
	if runtime.GOOS != "linux" || runtime.GOARCH != "amd64" {
		t.Skip("This native verification lane currently supplies Linux x64 official archives")
	}
	pkg := os.Getenv("LITHE_MANAGED_TEST_PACKAGE")
	if pkg == "" {
		t.Fatal("Run scripts/package/test-host-managed.mjs with a built Java plugin ZIP")
	}
	manifest, err := os.ReadFile(filepath.Join(pkg, "plugin.json"))
	if err != nil {
		t.Fatal(err)
	}
	var declaration map[string]any
	if err := json.Unmarshal(manifest, &declaration); err != nil {
		t.Fatal(err)
	}
	pluginID, version := declaration["id"].(string), declaration["version"].(string)
	ctx, cancel := context.WithTimeout(context.Background(), 180*time.Second)
	defer cancel()
	kernel := core.NewClient()
	plan, err := kernel.PlanManagedRuntimes(ctx, manifest, core.ManagedRuntimePlatform{OS: "linux", Arch: "x86_64"})
	if err != nil {
		t.Fatal(err)
	}
	paths := map[string]string{
		"jdk":   os.Getenv("LITHE_MANAGED_TEST_JDK_ARCHIVE"),
		"jdtls": os.Getenv("LITHE_MANAGED_TEST_JDTLS_ARCHIVE"),
	}
	byURL := make(map[string]string)
	for _, dependency := range plan.Dependencies {
		path := paths[dependency.ID]
		file, err := os.Open(path)
		if err != nil {
			t.Fatal(err)
		}
		hash := sha256.New()
		_, readErr := io.Copy(hash, file)
		closeErr := file.Close()
		if readErr != nil || closeErr != nil || hex.EncodeToString(hash.Sum(nil)) != dependency.SHA256 {
			t.Fatalf("Official archive checksum mismatch for %s", dependency.ID)
		}
		byURL[dependency.URL] = dependency.ID
	}
	service, err := pluginservice.Open(kernel, store.At(t.TempDir()), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	service.SetRuntimeLanguage("zh-CN")
	var lock sync.Mutex
	downloads := make(map[string]int)
	service.RuntimeDownloader = marketplace.NewArchiveDownloader(&http.Client{Transport: javaArchiveTransport(func(request *http.Request) (*http.Response, error) {
		id, exists := byURL[request.URL.String()]
		if !exists {
			return nil, fmt.Errorf("unexpected archive request")
		}
		file, err := os.Open(paths[id])
		if err != nil {
			return nil, err
		}
		info, err := file.Stat()
		if err != nil {
			_ = file.Close()
			return nil, err
		}
		lock.Lock()
		downloads[id]++
		lock.Unlock()
		return &http.Response{StatusCode: http.StatusOK, Header: make(http.Header), Body: file, ContentLength: info.Size(), Request: request}, nil
	})})
	service.RuntimeIssue = func(_ context.Context, failure error) { t.Logf("Runtime recovery category: %T", failure) }
	first, err := service.EnsureRuntimes(ctx, manifest, pkg, pluginID, version, true, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(downloads, map[string]int{"jdk": 1, "jdtls": 1}) {
		t.Fatalf("Expected a clean runner with both dependencies absent; transfers: %v", downloads)
	}
	if len(first.Runtimes) != 2 || len(first.Tools) != 1 || first.Tools["jdtls-runtime"].Path == "" {
		t.Fatalf("incomplete service environment: %+v", first)
	}
	if _, exists := first.Tools["java"]; exists {
		t.Fatal("managed service JDK changed the project java tool")
	}
	for _, installation := range first.Runtimes {
		if filepath.Dir(installation.Home) == pkg {
			t.Fatal("runtime was written inside the plugin package")
		}
	}
	second, err := service.EnsureRuntimes(ctx, manifest, pkg, pluginID, version, false, nil)
	if err != nil || !reflect.DeepEqual(first.Runtimes, second.Runtimes) {
		t.Fatalf("startup did not reuse prepared paths: %v", err)
	}
	// Only the plugin version changes: the runtime artifact identities do not.
	declaration["version"] = "99.0.0"
	updated, err := json.Marshal(declaration)
	if err != nil {
		t.Fatal(err)
	}
	third, err := service.EnsureRuntimes(ctx, updated, pkg, pluginID, "99.0.0", true, nil)
	if err != nil || !reflect.DeepEqual(first.Runtimes, third.Runtimes) {
		t.Fatalf("plugin update did not reuse prepared paths: %v", err)
	}
	if !reflect.DeepEqual(downloads, map[string]int{"jdk": 1, "jdtls": 1}) {
		t.Fatalf("startup or update downloaded a second copy: %v", downloads)
	}
	// Damage only the isolated test cache, retaining the verified source archive.
	oldJDK := first.Runtimes["jdk"].Home
	if err := os.Remove(filepath.Join(oldJDK, "bin", "javac")); err != nil {
		t.Fatal(err)
	}
	repaired, err := service.EnsureRuntimes(ctx, updated, pkg, pluginID, "99.0.0", false, nil)
	if err != nil {
		t.Fatal(err)
	}
	if repaired.Runtimes["jdk"].Home == oldJDK || repaired.Runtimes["jdtls"] != first.Runtimes["jdtls"] {
		t.Fatal("repair did not replace just the damaged dependency")
	}
	if !reflect.DeepEqual(downloads, map[string]int{"jdk": 1, "jdtls": 1}) {
		t.Fatalf("repair should reuse verified archive bytes without another transfer: %v", downloads)
	}
	after, err := os.ReadFile(filepath.Join(pkg, "plugin.json"))
	if err != nil || string(after) != string(manifest) {
		t.Fatal("preparation changed the installed plugin manifest")
	}
	t.Log("Real Java backend + Core + host: official archives verified/extracted, startup/update zero transfers, separate service tool, damaged JDK repaired from cached archive")
}
