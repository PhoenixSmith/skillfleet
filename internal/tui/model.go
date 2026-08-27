package tui

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"unicode/utf8"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

type ViewTab int

const (
	SkillsTab ViewTab = iota
	EndpointsTab
	PlanTab
)

var tabNames = []string{"Skills", "Endpoints", "Plan"}

type formState struct {
	mode, name, path, editing string
	field, preset             int
	vacuum                    bool
}

type Model struct {
	snapshot                   Snapshot
	width, height              int
	tab                        ViewTab
	cursor, endpointCursor     int
	query                      string
	searching, showHelp        bool
	err                        error
	runner                     Runner
	changes                    []Change
	routeEdits                 map[string]map[string]bool
	endpointEdits              map[string]*Endpoint
	form                       *formState
	plan                       *ApplyPlan
	conflictCursor             int
	dirtyQuit, externalWarning bool
	modalTitle, modalBody      string
	fingerprint                string
}
type reloadMsg struct {
	snapshot Snapshot
	err      error
}

var (
	accent        = lipgloss.Color("63")
	muted         = lipgloss.Color("245")
	good          = lipgloss.Color("42")
	warn          = lipgloss.Color("214")
	bad           = lipgloss.Color("196")
	titleStyle    = lipgloss.NewStyle().Bold(true).Foreground(accent)
	selectedStyle = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("230")).Background(accent)
	mutedStyle    = lipgloss.NewStyle().Foreground(muted)
)

func NewModel(s Snapshot) Model {
	m := Model{snapshot: s, width: 100, height: 30, runner: CLIRunner{}, routeEdits: map[string]map[string]bool{}, endpointEdits: map[string]*Endpoint{}, fingerprint: fileFingerprint(s.ConfigPath)}
	m.restoreDraft()
	return m
}
func NewModelWithRunner(s Snapshot, r Runner) Model { m := NewModel(s); m.runner = r; return m }
func (m Model) Init() tea.Cmd                       { return nil }
func loadCmd(p string) tea.Cmd {
	return func() tea.Msg { s, e := LoadSnapshot(p); return reloadMsg{s, e} }
}
func (m Model) dirty() bool { return len(m.changes) > 0 }
func (m *Model) addChange(c Change) {
	for i, x := range m.changes {
		if x.Kind == c.Kind && x.Name == c.Name {
			m.changes[i] = c
			m.saveDraft()
			return
		}
	}
	m.changes = append(m.changes, c)
	m.saveDraft()
}

type diskDraft struct {
	ConfigPath, Fingerprint string
	Changes                 []Change
}

func draftPath() string {
	h, e := os.UserHomeDir()
	if e != nil {
		return ""
	}
	return filepath.Join(h, ".cache", "skillfleet", "draft.json")
}
func (m *Model) saveDraft() {
	p := draftPath()
	if p == "" {
		return
	}
	os.MkdirAll(filepath.Dir(p), 0700)
	if len(m.changes) == 0 {
		os.Remove(p)
		return
	}
	b, e := json.Marshal(diskDraft{m.snapshot.ConfigPath, m.fingerprint, m.changes})
	if e == nil {
		_ = os.WriteFile(p, b, 0600)
	}
}
func (m *Model) restoreDraft() {
	b, e := os.ReadFile(draftPath())
	if e != nil {
		return
	}
	var d diskDraft
	if json.Unmarshal(b, &d) != nil || d.ConfigPath != m.snapshot.ConfigPath || d.Fingerprint != m.fingerprint {
		return
	}
	m.changes = d.Changes
	for _, c := range d.Changes {
		switch c.Kind {
		case ChangeRoute:
			x := map[string]bool{}
			for _, n := range c.Targets {
				x[n] = true
			}
			m.routeEdits[c.Name] = x
		case ChangeEndpointAdd, ChangeEndpointEdit:
			m.endpointEdits[c.Name] = &Endpoint{Path: c.Path, Vacuum: c.Vacuum}
		case ChangeEndpointRemove:
			m.endpointEdits[c.Name] = nil
		}
	}
}
func (m Model) endpointNames() []string {
	set := map[string]bool{}
	for n := range m.snapshot.Config.Endpoints {
		set[n] = true
	}
	for n, e := range m.endpointEdits {
		if e == nil {
			delete(set, n)
		} else {
			set[n] = true
		}
	}
	out := make([]string, 0, len(set))
	for n := range set {
		out = append(out, n)
	}
	sort.Strings(out)
	return out
}
func (m Model) targets(skill string) map[string]bool {
	if x, ok := m.routeEdits[skill]; ok {
		return x
	}
	out := map[string]bool{}
	for _, n := range m.snapshot.Config.Skills[skill].Targets {
		out[n] = true
	}
	return out
}
func (m *Model) toggleRoute() {
	names := m.filtered()
	eps := m.endpointNames()
	if len(names) == 0 || len(eps) == 0 {
		return
	}
	skill, ep := names[m.cursor], eps[min(m.endpointCursor, len(eps)-1)]
	t := m.targets(skill)
	cp := map[string]bool{}
	for k, v := range t {
		cp[k] = v
	}
	cp[ep] = !cp[ep]
	m.routeEdits[skill] = cp
	var ts []string
	for n, v := range cp {
		if v {
			ts = append(ts, n)
		}
	}
	sort.Strings(ts)
	m.addChange(Change{Kind: ChangeRoute, Name: skill, Targets: ts})
}
func (m *Model) removeEndpoint(name string) {
	var deps []string
	for _, s := range m.snapshot.SkillNames() {
		if m.targets(s)[name] {
			deps = append(deps, s)
		}
	}
	if len(deps) > 0 {
		m.modalTitle = "Endpoint has routes"
		m.modalBody = "Move or remove these routes first: " + strings.Join(deps, ", ") + "\nUse Space in Skills, then remove again."
		return
	}
	m.endpointEdits[name] = nil
	m.addChange(Change{Kind: ChangeEndpointRemove, Name: name})
}
func (m Model) buildPlan() ApplyPlan {
	p := ApplyPlan{}
	for _, c := range m.changes {
		switch c.Kind {
		case ChangeEndpointRemove:
			p.Removes = append(p.Removes, "endpoint "+c.Name)
		case ChangeEndpointAdd, ChangeEndpointEdit:
			p.Creates = append(p.Creates, c.Label()+" → "+c.Path)
		case ChangeSkillAdd:
			p.Creates = append(p.Creates, c.Label()+" ← "+c.Path)
		case ChangeRoute:
			old := map[string]bool{}
			skill, skillExists := m.snapshot.Config.Skills[c.Name]
			for _, x := range skill.Targets {
				old[x] = true
			}
			now := map[string]bool{}
			for _, x := range c.Targets {
				now[x] = true
				if !old[x] {
					dst := ""
					if ep, ok := m.snapshot.Config.Endpoints[x]; ok {
						dst = expand(ep.Path) + "/" + c.Name
					}
					state := RouteMissing
					if skillExists {
						state = inspectRoute(sourcePath(m.snapshot.Config, skill, x), dst)
					}
					if state == RouteConflict || state == RouteBroken {
						p.Conflicts = append(p.Conflicts, Conflict{Skill: c.Name, Endpoint: x, Destination: dst})
					} else {
						p.Creates = append(p.Creates, c.Name+" → "+x)
					}
				}
			}
			for x := range old {
				if !now[x] {
					p.Removes = append(p.Removes, c.Name+" → "+x)
				}
			}
		}
	}
	return p
}
func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch x := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = x.Width, x.Height
	case reloadMsg:
		m.err = x.err
		if x.err == nil {
			m.snapshot = x.snapshot
			m.fingerprint = fileFingerprint(m.snapshot.ConfigPath)
		}
	case applyResultMsg:
		m.modalTitle = "Apply summary"
		m.modalBody = x.output
		if x.err != nil {
			m.modalBody = "ERROR: " + x.err.Error() + "\n" + m.modalBody
		} else {
			m.changes = nil
			m.routeEdits = map[string]map[string]bool{}
			m.endpointEdits = map[string]*Endpoint{}
			m.plan = nil
			m.saveDraft()
			return m, loadCmd(m.snapshot.ConfigPath)
		}
	case tea.KeyMsg:
		k := x.String()
		if m.modalTitle != "" {
			m.modalTitle, m.modalBody = "", ""
			return m, nil
		}
		if m.dirtyQuit {
			if k == "y" {
				return m, tea.Quit
			}
			if k == "n" || k == "esc" {
				m.dirtyQuit = false
			}
			return m, nil
		}
		if m.externalWarning {
			if k == "r" {
				m.externalWarning = false
				m.changes = nil
				m.routeEdits = map[string]map[string]bool{}
				return m, loadCmd(m.snapshot.ConfigPath)
			}
			if k == "esc" {
				m.externalWarning = false
			}
			return m, nil
		}
		if m.form != nil {
			return m.updateForm(x)
		}
		if m.plan != nil {
			return m.updatePlan(x)
		}
		if m.searching {
			return m.updateSearch(x)
		}
		if k == "tab" {
			m.tab = (m.tab + 1) % 3
			m.cursor = 0
			return m, nil
		}
		if k == "shift+tab" {
			m.tab = (m.tab + 2) % 3
			m.cursor = 0
			return m, nil
		}
		switch k {
		case "q", "ctrl+c":
			if m.dirty() {
				m.dirtyQuit = true
			} else {
				return m, tea.Quit
			}
		case "esc":
			m.showHelp = false
		case "?":
			m.showHelp = !m.showHelp
		case "/":
			if m.tab == SkillsTab {
				m.searching = true
			}
		case "ctrl+s":
			if m.dirty() {
				p := m.buildPlan()
				m.plan = &p
				m.tab = PlanTab
			}
		case "r":
			if fileFingerprint(m.snapshot.ConfigPath) != m.fingerprint && m.dirty() {
				m.externalWarning = true
			} else {
				return m, loadCmd(m.snapshot.ConfigPath)
			}
		case "up", "k":
			if m.cursor > 0 {
				m.cursor--
			}
		case "down", "j":
			if m.cursor < m.itemCount()-1 {
				m.cursor++
			}
		case "left", "h":
			if m.endpointCursor > 0 {
				m.endpointCursor--
			}
		case "right", "l":
			if m.endpointCursor < len(m.endpointNames())-1 {
				m.endpointCursor++
			}
		case " ":
			if m.tab == SkillsTab {
				m.toggleRoute()
			}
		case "a":
			if m.tab == EndpointsTab {
				m.form = &formState{mode: "Add endpoint", vacuum: true}
			}
		case "n":
			if m.tab == SkillsTab {
				m.form = &formState{mode: "Add skill"}
			}
		case "e":
			if m.tab == EndpointsTab {
				ns := m.endpointNames()
				if len(ns) > 0 {
					n := ns[m.cursor]
					ep := m.snapshot.Config.Endpoints[n]
					if staged, ok := m.endpointEdits[n]; ok && staged != nil {
						ep = *staged
					}
					m.form = &formState{mode: "Edit endpoint", name: n, path: ep.Path, editing: n, field: 1, vacuum: vacuumEnabled(ep)}
				}
			}
		case "d", "delete":
			if m.tab == EndpointsTab {
				ns := m.endpointNames()
				if len(ns) > 0 {
					m.removeEndpoint(ns[m.cursor])
				}
			}
		}
	}
	return m, nil
}
func (m Model) itemCount() int {
	if m.tab == SkillsTab {
		return len(m.filtered())
	}
	if m.tab == EndpointsTab {
		return len(m.endpointNames())
	}
	return len(m.changes)
}
func (m Model) updateSearch(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch k.String() {
	case "esc":
		m.searching = false
		m.query = ""
	case "enter":
		m.searching = false
	case "backspace":
		if len(m.query) > 0 {
			_, n := utf8.DecodeLastRuneInString(m.query)
			m.query = m.query[:len(m.query)-n]
		}
	default:
		if len(k.Runes) > 0 {
			m.query += string(k.Runes)
		}
	}
	m.cursor = 0
	return m, nil
}
func (m Model) updateForm(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	f := m.form
	switch k.String() {
	case "esc":
		m.form = nil
	case "tab", "down":
		fields := 2
		if f.mode != "Add skill" {
			fields = 3
		}
		f.field = (f.field + 1) % fields
	case "shift+tab", "up":
		fields := 2
		if f.mode != "Add skill" {
			fields = 3
		}
		f.field = (f.field + fields - 1) % fields
	case " ":
		if f.mode != "Add skill" && f.field == 2 {
			f.vacuum = !f.vacuum
		}
	case "backspace":
		p := &f.name
		if f.field == 1 {
			p = &f.path
		}
		if len(*p) > 0 {
			_, n := utf8.DecodeLastRuneInString(*p)
			*p = (*p)[:len(*p)-n]
		}
	case "ctrl+p":
		if f.mode == "Add endpoint" {
			presets := [][2]string{{"hermes", "~/.hermes/skills"}, {"claude", "~/.claude/skills"}, {"codex", "~/.codex/skills"}, {"pi", "~/.pi/agent/skills"}}
			p := presets[f.preset%len(presets)]
			f.preset++
			f.name, f.path = p[0], p[1]
		}
	case "enter":
		if f.mode == "Add skill" {
			if strings.TrimSpace(f.name) == "" || strings.TrimSpace(f.path) == "" {
				m.modalTitle = "Invalid skill"
				m.modalBody = "Name and library-relative source are required."
				return m, nil
			}
			if _, ok := m.snapshot.Config.Skills[f.name]; ok {
				m.modalTitle = "Invalid skill"
				m.modalBody = "Skill already exists."
				return m, nil
			}
			m.addChange(Change{Kind: ChangeSkillAdd, Name: f.name, Path: f.path})
			m.modalTitle = "Skill staged"
			m.modalBody = "Skill is now in the matrix. Close this message, select it, and route it with Space before the same apply."
			m.form = nil
			m.query = ""
			for i, n := range m.filtered() {
				if n == f.name {
					m.cursor = i
					break
				}
			}
			return m, nil
		}
		if err := validateEndpoint(f.name, f.path, m.snapshot.Config, f.editing); err != nil {
			m.modalTitle = "Invalid endpoint"
			m.modalBody = err.Error()
		} else {
			kind := ChangeEndpointAdd
			if f.editing != "" {
				kind = ChangeEndpointEdit
			}
			m.endpointEdits[f.name] = &Endpoint{Path: f.path, Vacuum: boolPtr(f.vacuum)}
			m.addChange(Change{Kind: kind, Name: f.name, Path: f.path, Vacuum: boolPtr(f.vacuum)})
			m.modalTitle = "Endpoint staged"
			m.modalBody = fmt.Sprintf("%s\n%s. No files changed until Ctrl+S.", expand(f.path), plural(unmanagedCount(f.path, m.snapshot.Config.Skills), "unmanaged entry found", "unmanaged entries found"))
			m.form = nil
		}
	default:
		if len(k.Runes) > 0 {
			if f.field == 0 {
				f.name += string(k.Runes)
			} else {
				f.path += string(k.Runes)
			}
		}
	}
	return m, nil
}
func (m Model) updatePlan(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	if k.String() == "esc" {
		m.plan = nil
		return m, nil
	}
	if len(m.plan.Conflicts) > 0 {
		switch k.String() {
		case "up":
			if m.conflictCursor > 0 {
				m.conflictCursor--
			}
		case "down":
			if m.conflictCursor < len(m.plan.Conflicts)-1 {
				m.conflictCursor++
			}
		case "s":
			m.plan.Conflicts[m.conflictCursor].Resolution = "skip"
		case "k":
			m.plan.Conflicts[m.conflictCursor].Resolution = "keep existing"
		case "b":
			m.plan.Conflicts[m.conflictCursor].Resolution = "backup+link"
		}
	}
	if k.String() == "enter" {
		force := false
		for _, c := range m.plan.Conflicts {
			if c.Resolution == "" {
				m.modalTitle = "Unresolved conflicts"
				m.modalBody = "Choose skip, keep existing, or backup+link for every conflict."
				return m, nil
			}
			if c.Resolution == "backup+link" {
				force = true
			}
		}
		changes := append([]Change(nil), m.changes...)
		for _, c := range m.plan.Conflicts {
			if c.Resolution == "skip" || c.Resolution == "keep existing" {
				for i := range changes {
					if changes[i].Kind == ChangeRoute && changes[i].Name == c.Skill {
						var ts []string
						for _, e := range changes[i].Targets {
							if e != c.Endpoint {
								ts = append(ts, e)
							}
						}
						changes[i].Targets = ts
					}
				}
			}
		}
		return m, runApply(m.runner, m.snapshot.ConfigPath, changes, force)
	}
	return m, nil
}
func (m Model) filtered() []string {
	set := map[string]bool{}
	for _, n := range m.snapshot.SkillNames() {
		set[n] = true
	}
	for _, c := range m.changes {
		if c.Kind == ChangeSkillAdd {
			set[c.Name] = true
		}
	}
	ns := make([]string, 0, len(set))
	for n := range set {
		ns = append(ns, n)
	}
	sort.Strings(ns)
	if m.query == "" {
		return ns
	}
	var out []string
	q := strings.ToLower(m.query)
	for _, n := range ns {
		if strings.Contains(strings.ToLower(n), q) {
			out = append(out, n)
		}
	}
	return out
}
func badge(s RouteState) string {
	c := good
	if s != RouteOK {
		c = warn
	}
	if s == RouteConflict || s == RouteBroken {
		c = bad
	}
	return lipgloss.NewStyle().Foreground(c).Render(strings.ToUpper(string(s)))
}
func clamp(s string, n int) string {
	if n < 2 {
		return ""
	}
	if lipgloss.Width(s) <= n {
		return s
	}
	r := []rune(s)
	for len(r) > 0 && lipgloss.Width(string(r))+1 > n {
		r = r[:len(r)-1]
	}
	return string(r) + "…"
}
func (m Model) header() string {
	var ts []string
	for i, n := range tabNames {
		if ViewTab(i) == m.tab {
			ts = append(ts, selectedStyle.Render(" "+n+" "))
		} else {
			ts = append(ts, n)
		}
	}
	dirty := ""
	if m.dirty() {
		dirty = fmt.Sprintf("  %d staged", len(m.changes))
	}
	return titleStyle.Render("SKILLFLEET") + dirty + "\n" + strings.Join(ts, "   ")
}
func padCell(s string, n int) string {
	if d := n - lipgloss.Width(s); d > 0 {
		return s + strings.Repeat(" ", d)
	}
	return s
}

// vacuumPreview lists manual skills that the post-apply sync would adopt,
// as "skill ← endpoint", honoring staged endpoint edits and skill adds.
func (m Model) vacuumPreview() []string {
	declared := map[string]bool{}
	for _, n := range m.snapshot.SkillNames() {
		declared[n] = true
	}
	for _, c := range m.changes {
		if c.Kind == ChangeSkillAdd {
			declared[c.Name] = true
		}
	}
	var out []string
	for _, n := range m.endpointNames() {
		ep, ok := m.snapshot.Config.Endpoints[n]
		if x, yes := m.endpointEdits[n]; yes && x != nil {
			ep, ok = *x, true
		}
		if !ok || !vacuumEnabled(ep) {
			continue
		}
		for _, s := range vacuumCandidates(ep.Path, declared) {
			out = append(out, s+" ← "+n)
		}
	}
	return out
}

func (m Model) skillsView(w, h int) string {
	ns := m.filtered()
	eps := m.endpointNames()
	lines := []string{titleStyle.Render("Skills  ←/→ endpoint, Space toggle")}
	if m.searching || m.query != "" {
		lines = append(lines, "/ "+m.query)
	}
	nameW := lipgloss.Width("Skill")
	for _, n := range ns {
		if x := lipgloss.Width(n); x > nameW {
			nameW = x
		}
	}
	colW := make([]int, len(eps))
	head := padCell("Skill", nameW)
	for j, e := range eps {
		colW[j] = max(lipgloss.Width(e), 3)
		head += "  " + padCell(e, colW[j])
	}
	lines = append(lines, clamp("  "+head, w))
	if len(ns) == 0 {
		if m.query != "" {
			lines = append(lines, "No skills match “"+m.query+"”.")
		} else {
			lines = append(lines, "No skills. Press n to add one.")
		}
		return strings.Join(lines, "\n")
	}
	avail := max(1, h-len(lines)-2)
	start := max(0, m.cursor-avail+1)
	end := min(len(ns), start+avail)
	if start > 0 {
		lines = append(lines, mutedStyle.Render(fmt.Sprintf("  ↑ %d more", start)))
	}
	for i := start; i < end; i++ {
		n := ns[i]
		row := padCell(n, nameW)
		for j, e := range eps {
			mark := "☐"
			if m.targets(n)[e] {
				mark = "☑"
			}
			if i == m.cursor && j == m.endpointCursor {
				mark = "[" + mark + "]"
			}
			row += "  " + padCell(mark, colW[j])
		}
		if i == m.cursor {
			row = "› " + row
		} else {
			row = "  " + row
		}
		lines = append(lines, clamp(row, w))
	}
	if end < len(ns) {
		lines = append(lines, mutedStyle.Render(fmt.Sprintf("  ↓ %d more", len(ns)-end)))
	}
	return strings.Join(lines, "\n")
}
func (m Model) endpointsView(w int) string {
	ns := m.endpointNames()
	lines := []string{titleStyle.Render("Endpoints")}
	for i, n := range ns {
		ep, ok := m.snapshot.Config.Endpoints[n]
		if x, yes := m.endpointEdits[n]; yes && x != nil {
			ep = *x
			ok = true
		}
		if !ok {
			continue
		}
		routes := 0
		for _, s := range m.snapshot.SkillNames() {
			if m.targets(s)[n] {
				routes++
			}
		}
		prefix := "  "
		if i == m.cursor {
			prefix = "› "
		}
		vacuum := "vacuum off"
		if vacuumEnabled(ep) {
			vacuum = "vacuum on"
		}
		line := fmt.Sprintf("%s%s  %s  %d routes  %d unmanaged  %s", prefix, n, vacuum, routes, unmanagedCount(ep.Path, m.snapshot.Config.Skills), expand(ep.Path))
		lines = append(lines, clamp(line, w))
	}
	if len(ns) == 0 {
		lines = append(lines, "No endpoints. Press a to add one.")
	}
	return strings.Join(lines, "\n")
}
func (m Model) planView(w int) string {
	p := m.buildPlan()
	if m.plan != nil {
		p = *m.plan
	}
	lines := []string{titleStyle.Render("Apply review")}
	vac := m.vacuumPreview()
	if len(m.changes) == 0 {
		lines = append(lines, "No staged changes.")
		lines = append(lines, vacuumLines(vac, w)...)
		return strings.Join(lines, "\n")
	}
	lines = append(lines, "Creates / changes:")
	for _, x := range p.Creates {
		lines = append(lines, "  + "+clamp(x, w-4))
	}
	lines = append(lines, "Removes:")
	for _, x := range p.Removes {
		lines = append(lines, "  - "+clamp(x, w-4))
	}
	lines = append(lines, "Conflicts:")
	for i, c := range p.Conflicts {
		mark := "  "
		if i == m.conflictCursor {
			mark = "› "
		}
		r := c.Resolution
		if r == "" {
			r = "unresolved"
		}
		lines = append(lines, clamp(fmt.Sprintf("%s%s → %s [%s]", mark, c.Skill, c.Endpoint, r), w))
	}
	lines = append(lines, vacuumLines(vac, w)...)
	if m.plan != nil {
		lines = append(lines, "", "Enter apply safely · Esc back")
	}
	return strings.Join(lines, "\n")
}
func vacuumLines(vac []string, w int) []string {
	if len(vac) == 0 {
		return nil
	}
	lines := []string{fmt.Sprintf("Vacuum — sync will adopt %d manual skill(s) into the library:", len(vac))}
	for _, v := range vac {
		lines = append(lines, "  ~ "+clamp(v, w-4))
	}
	return lines
}
func (m Model) overlay() string {
	if m.modalTitle != "" {
		return titleStyle.Render(m.modalTitle) + "\n\n" + m.modalBody + "\n\nPress any key"
	}
	if m.dirtyQuit {
		return titleStyle.Render("Discard staged changes and quit?") + "\n\ny yes   n no"
	}
	if m.externalWarning {
		return titleStyle.Render("Config changed outside Skillfleet") + "\n\nr reload and discard draft   Esc keep staged draft"
	}
	if m.form != nil {
		f := m.form
		label := "Path"
		if f.mode == "Add skill" {
			label = "Source"
		}
		vacuum := ""
		if f.mode != "Add skill" {
			mark := "☐"
			if f.vacuum {
				mark = "☑"
			}
			vacuum = fmt.Sprintf("\n%s Vacuum manual skills: %s", pick(f.field == 2, "›", " "), mark)
		}
		return fmt.Sprintf("%s\n\n%s Name: %s\n%s %s: %s%s\n\nTab next · Space toggle · Enter validate and stage · Esc cancel", titleStyle.Render(f.mode), pick(f.field == 0, "›", " "), f.name, pick(f.field == 1, "›", " "), label, f.path, vacuum)
	}
	return ""
}
func plural(n int, one, many string) string {
	if n == 1 {
		return "1 " + one
	}
	return fmt.Sprintf("%d %s", n, many)
}
func pick(ok bool, a, b string) string {
	if ok {
		return a
	}
	return b
}
func (m Model) footer() string {
	if m.showHelp {
		return "Tab/Shift+Tab views · arrows move · Space route · n skill · a/e/d endpoint · Ctrl+S review/apply · r reload · q quit · conflicts: s skip, k keep, b backup+link"
	}
	return mutedStyle.Render("Tab views  ↑↓ move  Space toggle  Ctrl+S review  ? help")
}
func (m Model) View() string {
	if m.width <= 0 {
		return ""
	}
	body := ""
	if o := m.overlay(); o != "" {
		body = o
	} else {
		switch m.tab {
		case SkillsTab:
			body = m.skillsView(m.width-4, max(5, m.height-7))
		case EndpointsTab:
			body = m.endpointsView(m.width - 4)
		case PlanTab:
			body = m.planView(m.width - 4)
		}
	}
	return clampLines(m.header()+"\n\n"+body+"\n\n"+m.footer(), m.width)
}
func clampLines(s string, w int) string {
	ls := strings.Split(s, "\n")
	for i := range ls {
		ls[i] = clamp(ls[i], w)
	}
	return strings.Join(ls, "\n")
}
func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
