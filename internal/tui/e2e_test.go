package tui

import (
	"os"
	"path/filepath"
	"testing"
)

// TestRealCLIEndToEnd exercises the same staged apply pipeline as the TUI
// against the real Rust binary and an isolated config. It is opt-in so normal
// Go-only development does not require Cargo.
func TestRealCLIEndToEnd(t *testing.T) {
	bin := os.Getenv("SKILLFLEET_E2E_BIN")
	if bin == "" {
		t.Skip("set SKILLFLEET_E2E_BIN to the Rust binary")
	}
	root := t.TempDir()
	lib := filepath.Join(root, "library")
	ep1 := filepath.Join(root, "ep1")
	ep2 := filepath.Join(root, "ep2")
	for _, p := range []string{filepath.Join(lib, "skills", "existing"), filepath.Join(lib, "skills", "added"), ep1, ep2} {
		if err := os.MkdirAll(p, 0755); err != nil {
			t.Fatal(err)
		}
	}
	for _, n := range []string{"existing", "added"} {
		if err := os.WriteFile(filepath.Join(lib, "skills", n, "SKILL.md"), []byte("# "+n), 0644); err != nil {
			t.Fatal(err)
		}
	}
	cfg := filepath.Join(root, "skillfleet.toml")
	text := "schema = 1\nlibrary = \"" + lib + "\"\n\n[endpoints.one]\npath = \"" + ep1 + "\"\n\n[skills.existing]\nsource = \"skills/existing\"\ntargets = []\n"
	if err := os.WriteFile(cfg, []byte(text), 0644); err != nil {
		t.Fatal(err)
	}
	changes := []Change{{Kind: ChangeEndpointAdd, Name: "two", Path: ep2}, {Kind: ChangeSkillAdd, Name: "added", Path: "skills/added"}, {Kind: ChangeRoute, Name: "added", Targets: []string{"two"}}, {Kind: ChangeRoute, Name: "existing", Targets: []string{"one", "two"}}}
	msg := runApply(CLIRunner{Binary: bin}, cfg, changes, false)().(applyResultMsg)
	if msg.err != nil {
		t.Fatalf("apply: %v\n%s", msg.err, msg.output)
	}
	s, err := LoadSnapshot(cfg)
	if err != nil {
		t.Fatal(err)
	}
	if len(s.Config.Endpoints) != 2 || len(s.Config.Skills) != 2 {
		t.Fatalf("config counts endpoints=%d skills=%d", len(s.Config.Endpoints), len(s.Config.Skills))
	}
	for _, p := range []string{filepath.Join(ep2, "added"), filepath.Join(ep1, "existing"), filepath.Join(ep2, "existing")} {
		info, err := os.Lstat(p)
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode()&os.ModeSymlink == 0 {
			t.Fatalf("not a symlink: %s", p)
		}
	}
	if c := s.StateCounts(); c[RouteOK] != 3 {
		t.Fatalf("doctor-equivalent route state=%v\n%s", c, msg.output)
	}
}
