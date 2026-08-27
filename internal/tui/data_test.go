package tui

import (
	"os"
	"path/filepath"
	"testing"
)

func TestDefaultConfigPathRepoLocal(t *testing.T) {
	defer os.Unsetenv("SKILLFLEET_CONFIG")
	os.Unsetenv("SKILLFLEET_CONFIG")

	base := t.TempDir()
	nested := filepath.Join(base, "a", "b")
	if err := os.MkdirAll(nested, 0o755); err != nil {
		t.Fatal(err)
	}
	origWd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	defer os.Chdir(origWd)

	// No manifest anywhere -> fall back to XDG under HOME.
	t.Setenv("HOME", base)
	if err := os.Chdir(nested); err != nil {
		t.Fatal(err)
	}
	if got := DefaultConfigPath(); got != filepath.Join(base, ".config", "skillfleet", "skillfleet.toml") {
		t.Fatalf("fallback = %q, want XDG", got)
	}

	// Manifest at a parent dir -> repo-local wins over XDG.
	manifest := filepath.Join(base, "skillfleet.toml")
	if err := os.WriteFile(manifest, []byte("schema = 1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if got := DefaultConfigPath(); !sameResolvedPath(got, manifest) {
		t.Fatalf("repo-local = %q, want %q", got, manifest)
	}

	// Closest manifest shadows further-up ones.
	nearer := filepath.Join(nested, "skillfleet.toml")
	if err := os.WriteFile(nearer, []byte("schema = 1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if got := DefaultConfigPath(); !sameResolvedPath(got, nearer) {
		t.Fatalf("closest = %q, want %q", got, nearer)
	}
}

// macOS exposes temporary directories through both /var and /private/var.
func sameResolvedPath(a, b string) bool {
	ra, errA := filepath.EvalSymlinks(a)
	rb, errB := filepath.EvalSymlinks(b)
	return errA == nil && errB == nil && ra == rb
}

func TestDefaultConfigPathEnvWins(t *testing.T) {
	t.Setenv("SKILLFLEET_CONFIG", "/custom/config.toml")
	if got := DefaultConfigPath(); got != "/custom/config.toml" {
		t.Fatalf("env override = %q, want /custom/config.toml", got)
	}
}
