package tui

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

func fixture(t *testing.T) Snapshot {
	t.Helper()
	root := t.TempDir()
	lib := filepath.Join(root, "library")
	ep := filepath.Join(root, "endpoint")
	os.MkdirAll(filepath.Join(lib, "skills", "alpha"), 0755)
	os.MkdirAll(ep, 0755)
	os.WriteFile(filepath.Join(lib, "skills", "alpha", "SKILL.md"), []byte("# alpha"), 0644)
	os.Symlink(filepath.Join(lib, "skills", "alpha"), filepath.Join(ep, "alpha"))
	cfg := filepath.Join(root, "skillfleet.toml")
	text := "schema = 1\nlibrary = \"" + lib + "\"\n\n[endpoints.hermes]\npath = \"" + ep + "\"\n\n[skills.alpha]\nsource = \"skills/alpha\"\ntargets = [\"hermes\"]\n\n[skills.beta]\nsource = \"skills/beta\"\ntargets = []\n"
	os.WriteFile(cfg, []byte(text), 0644)
	s, e := LoadSnapshot(cfg)
	if e != nil {
		t.Fatal(e)
	}
	return s
}
func key(m Model, s string) Model {
	var k tea.KeyMsg
	switch s {
	case "tab":
		k = tea.KeyMsg{Type: tea.KeyTab}
	case "shift+tab":
		k = tea.KeyMsg{Type: tea.KeyShiftTab}
	case " ":
		k = tea.KeyMsg{Type: tea.KeySpace}
	case "ctrl+s":
		k = tea.KeyMsg{Type: tea.KeyCtrlS}
	case "esc":
		k = tea.KeyMsg{Type: tea.KeyEsc}
	default:
		k = tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune(s)}
	}
	n, _ := m.Update(k)
	return n.(Model)
}
func TestLoadSnapshotInspectsFilesystem(t *testing.T) {
	c := fixture(t).StateCounts()
	if c[RouteOK] != 1 {
		t.Fatal(c)
	}
}
func TestTabsWrap(t *testing.T) {
	m := NewModel(fixture(t))
	m = key(m, "tab")
	if m.tab != EndpointsTab {
		t.Fatal(m.tab)
	}
	m = key(m, "tab")
	m = key(m, "tab")
	if m.tab != SkillsTab {
		t.Fatal(m.tab)
	}
	m = key(m, "shift+tab")
	if m.tab != PlanTab {
		t.Fatal(m.tab)
	}
}
func TestSpaceStagesFocusedRoute(t *testing.T) {
	m := NewModel(fixture(t))
	m.cursor = 1
	m = key(m, " ")
	if !m.targets("beta")["hermes"] || len(m.changes) != 1 {
		t.Fatalf("%v %#v", m.targets("beta"), m.changes)
	}
	m = key(m, " ")
	if m.targets("beta")["hermes"] {
		t.Fatal("toggle did not clear")
	}
}
func TestDependencyAwareEndpointRemove(t *testing.T) {
	m := NewModel(fixture(t))
	m.tab = EndpointsTab
	m.removeEndpoint("hermes")
	if len(m.changes) != 0 || m.modalTitle == "" {
		t.Fatal("dependent removal was not blocked")
	}
	m.modalTitle = ""
	m.routeEdits["alpha"] = map[string]bool{}
	m.removeEndpoint("hermes")
	if len(m.changes) != 1 {
		t.Fatal("removal not staged")
	}
}
func TestDirtyQuitWarns(t *testing.T) {
	m := NewModel(fixture(t))
	m.cursor = 1
	m = key(m, " ")
	m = key(m, "q")
	if !m.dirtyQuit {
		t.Fatal("no dirty warning")
	}
	m = key(m, "n")
	if m.dirtyQuit {
		t.Fatal("warning did not close")
	}
}
func TestViewsWideAndNarrowNoOverflow(t *testing.T) {
	lipgloss.SetColorProfile(0)
	for _, w := range []int{120, 54, 30} {
		m := NewModel(fixture(t))
		m.width = w
		m.height = 24
		for tab := SkillsTab; tab <= PlanTab; tab++ {
			m.tab = tab
			v := m.View()
			for _, line := range strings.Split(v, "\n") {
				if lipgloss.Width(line) > w {
					t.Fatalf("tab %d width %d > %d: %q", tab, lipgloss.Width(line), w, line)
				}
			}
		}
	}
}
func TestApplyPlanGroupsAndConflicts(t *testing.T) {
	s := fixture(t)
	ep := s.Config.Endpoints["hermes"].Path
	os.WriteFile(filepath.Join(ep, "beta"), []byte("mine"), 0644)
	m := NewModel(s)
	m.cursor = 1
	m = key(m, " ")
	p := m.buildPlan()
	if len(p.Conflicts) != 1 || p.Conflicts[0].Skill != "beta" {
		t.Fatalf("%#v", p)
	}
	m = key(m, "ctrl+s")
	if m.plan == nil || m.tab != PlanTab {
		t.Fatal("review not opened")
	}
	m = key(m, "enter")
	if m.modalTitle != "Unresolved conflicts" {
		t.Fatal("unresolved conflict was not blocked")
	}
}
func TestEndpointValidation(t *testing.T) {
	s := fixture(t)
	if err := validateEndpoint("copy", s.Config.Endpoints["hermes"].Path, s.Config, ""); err == nil {
		t.Fatal("duplicate path accepted")
	}
	p := filepath.Join(filepath.Dir(s.Config.Library), "new")
	os.Mkdir(p, 0755)
	if err := validateEndpoint("new", p, s.Config, ""); err != nil {
		t.Fatal(err)
	}
}

func TestNewSkillImmediatelyRoutableInSameDraft(t *testing.T) {
	m := NewModel(fixture(t))
	m.form = &formState{mode: "Add skill", name: "gamma", path: "skills/gamma"}
	n, _ := m.updateForm(tea.KeyMsg{Type: tea.KeyEnter})
	m = n.(Model)
	if got := m.filtered(); len(got) != 3 || got[2] != "gamma" { t.Fatalf("staged skill missing: %v", got) }
	m.modalTitle, m.modalBody = "", ""
	m.cursor = 2
	m.toggleRoute()
	if !m.targets("gamma")["hermes"] { t.Fatal("staged skill was not routable") }
	p := m.buildPlan()
	if len(p.Creates) < 2 { t.Fatalf("same-transaction plan=%#v", p) }
}
