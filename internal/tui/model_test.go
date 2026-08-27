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
	if got := m.filtered(); len(got) != 3 || got[2] != "gamma" {
		t.Fatalf("staged skill missing: %v", got)
	}
	m.modalTitle, m.modalBody = "", ""
	m.cursor = 2
	m.toggleRoute()
	if !m.targets("gamma")["hermes"] {
		t.Fatal("staged skill was not routable")
	}
	p := m.buildPlan()
	if len(p.Creates) < 2 {
		t.Fatalf("same-transaction plan=%#v", p)
	}
}

func TestEndpointVacuumCheckboxDefaultsOnAndStagesOptOut(t *testing.T) {
	m := NewModel(fixture(t))
	m.tab = EndpointsTab
	m = key(m, "a")
	if m.form == nil || !m.form.vacuum {
		t.Fatal("new endpoint vacuum checkbox should default on")
	}
	m.form.name = "local"
	m.form.path = t.TempDir()
	m.form.field = 2
	m = key(m, " ")
	if m.form.vacuum {
		t.Fatal("space did not disable vacuum")
	}
	m = key(m, "enter")
	if len(m.changes) != 1 || m.changes[0].Vacuum == nil || *m.changes[0].Vacuum {
		t.Fatalf("opt-out not staged: %#v", m.changes)
	}
}

func TestEndpointEditLoadsVacuumSettingAndViewShowsIt(t *testing.T) {
	s := fixture(t)
	ep := s.Config.Endpoints["hermes"]
	ep.Vacuum = boolPtr(false)
	s.Config.Endpoints["hermes"] = ep
	m := NewModel(s)
	m.tab = EndpointsTab
	m = key(m, "e")
	if m.form == nil || m.form.vacuum {
		t.Fatal("edit form did not load disabled vacuum state")
	}
}

func TestSkillsViewScrollsToKeepCursorVisible(t *testing.T) {
	s := fixture(t)
	for i := 0; i < 30; i++ {
		s.Config.Skills["zz-skill-"+string(rune('a'+i%26))+string(rune('a'+i/26))] = Skill{Source: "skills/x"}
	}
	m := NewModel(s)
	m.cursor = m.itemCount() - 1
	out := m.skillsView(120, 12)
	last := m.filtered()[m.cursor]
	if !strings.Contains(out, "› "+last) {
		t.Fatalf("cursor row not visible:\n%s", out)
	}
	if !strings.Contains(out, "↑") {
		t.Fatalf("missing overflow indicator:\n%s", out)
	}
	if got := len(strings.Split(out, "\n")); got > 14 {
		t.Fatalf("view overflows height: %d lines", got)
	}
}

func TestSkillsViewAlignsColumnsUnderEndpointNames(t *testing.T) {
	m := NewModel(fixture(t))
	out := strings.Split(m.skillsView(120, 20), "\n")
	var head, row string
	for _, l := range out {
		if strings.Contains(l, "Skill  ") && strings.Contains(l, "hermes") {
			head = l
		}
		if strings.Contains(l, "alpha") {
			row = l
		}
	}
	if head == "" || row == "" {
		t.Fatalf("missing header or row:\n%s", strings.Join(out, "\n"))
	}
	col := lipgloss.Width(head[:strings.Index(head, "hermes")])
	marks := lipgloss.Width(row[:strings.IndexAny(row, "☐☑[")])
	if col != marks {
		t.Fatalf("checkbox column %d not under endpoint header %d:\nhead: %q\nrow:  %q", marks, col, head, row)
	}
}

func TestSkillsViewEmptyStates(t *testing.T) {
	s := fixture(t)
	s.Config.Skills = map[string]Skill{}
	s.Routes = nil
	m := NewModel(s)
	if out := m.skillsView(80, 20); !strings.Contains(out, "No skills. Press n to add one.") {
		t.Fatalf("missing empty state:\n%s", out)
	}
	m2 := NewModel(fixture(t))
	m2.query = "zzz"
	if out := m2.skillsView(80, 20); !strings.Contains(out, "No skills match") {
		t.Fatalf("missing no-match state:\n%s", out)
	}
}

func TestPlanViewShowsPendingVacuumAdoptions(t *testing.T) {
	s := fixture(t)
	ep := s.Config.Endpoints["hermes"]
	manual := filepath.Join(expand(ep.Path), "manual-skill")
	os.MkdirAll(manual, 0755)
	os.WriteFile(filepath.Join(manual, "SKILL.md"), []byte("# m"), 0644)
	m := NewModel(s)
	out := m.planView(100)
	if !strings.Contains(out, "manual-skill ← hermes") || !strings.Contains(out, "sync will adopt") {
		t.Fatalf("vacuum preview missing:\n%s", out)
	}
	off := false
	s.Config.Endpoints["hermes"] = Endpoint{Path: ep.Path, Vacuum: &off}
	if out := NewModel(s).planView(100); strings.Contains(out, "manual-skill") {
		t.Fatalf("vacuum preview shown for opted-out endpoint:\n%s", out)
	}
}

type recordingRunner struct{ calls [][]string }

func (r *recordingRunner) Run(config string, args ...string) (string, error) {
	r.calls = append(r.calls, args)
	return "", nil
}

func TestApplySyncsBeforeEndpointRemovalToDetachLinks(t *testing.T) {
	r := &recordingRunner{}
	changes := []Change{
		{Kind: ChangeEndpointRemove, Name: "gone"},
		{Kind: ChangeRoute, Name: "alpha", Targets: []string{}},
	}
	runApply(r, "cfg.toml", changes, false)()
	var seq []string
	for _, c := range r.calls {
		seq = append(seq, strings.Join(c, " "))
	}
	got := strings.Join(seq, " | ")
	want := "skill route alpha --to | sync | endpoint remove gone | sync | doctor"
	if got != want {
		t.Fatalf("apply order wrong:\n got  %s\n want %s", got, want)
	}
}
