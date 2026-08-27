package tui

import (
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/BurntSushi/toml"
)

type Config struct {
	Schema    uint32              `toml:"schema"`
	Library   string              `toml:"library"`
	Endpoints map[string]Endpoint `toml:"endpoints"`
	Skills    map[string]Skill    `toml:"skills"`
}

type Endpoint struct {
	Path string `toml:"path"`
}
type Skill struct {
	Source          string            `toml:"source"`
	SourceOverrides map[string]string `toml:"source_overrides"`
	Targets         []string          `toml:"targets"`
	Remote          *Remote           `toml:"remote"`
}
type Remote struct{ Git, Subdir string }

type RouteState string

const (
	RouteOK       RouteState = "ok"
	RouteMissing  RouteState = "missing"
	RouteConflict RouteState = "conflict"
	RouteBroken   RouteState = "broken"
	RouteUnknown  RouteState = "unknown"
)

type Route struct {
	Skill, Endpoint, Source, Destination string
	State                                RouteState
}
type Snapshot struct {
	ConfigPath string
	Config     Config
	Routes     []Route
}

type ChangeKind int

const (
	ChangeRoute ChangeKind = iota
	ChangeEndpointAdd
	ChangeEndpointEdit
	ChangeEndpointRemove
	ChangeSkillAdd
)

type Change struct {
	Kind       ChangeKind
	Name, Path string
	Targets    []string
}

func (c Change) Label() string {
	switch c.Kind {
	case ChangeRoute:
		return "route " + c.Name
	case ChangeEndpointAdd:
		return "add endpoint " + c.Name
	case ChangeEndpointEdit:
		return "edit endpoint " + c.Name
	case ChangeSkillAdd:
		return "add skill " + c.Name
	default:
		return "remove endpoint " + c.Name
	}
}

type Conflict struct{ Skill, Endpoint, Destination, Resolution string }
type ApplyPlan struct {
	Creates, Removes []string
	Conflicts        []Conflict
}

func fileFingerprint(path string) string {
	b, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	return fmt.Sprintf("%x", sha256.Sum256(b))
}
func unmanagedCount(path string, skills map[string]Skill) int {
	es, err := os.ReadDir(expand(path))
	if err != nil {
		return 0
	}
	n := 0
	for _, e := range es {
		if _, ok := skills[e.Name()]; !ok {
			n++
		}
	}
	return n
}
func validateEndpoint(name, path string, c Config, editing string) error {
	name, path = strings.TrimSpace(name), filepath.Clean(expand(strings.TrimSpace(path)))
	if name == "" || path == "." {
		return fmt.Errorf("name and path are required")
	}
	if _, ok := c.Endpoints[name]; ok && name != editing {
		return fmt.Errorf("endpoint %q already exists", name)
	}
	lib := filepath.Clean(expand(c.Library))
	if path == lib || strings.HasPrefix(path, lib+string(os.PathSeparator)) || strings.HasPrefix(lib, path+string(os.PathSeparator)) {
		return fmt.Errorf("endpoint cannot be the library or nested with it")
	}
	for n, ep := range c.Endpoints {
		if n == editing {
			continue
		}
		p := filepath.Clean(expand(ep.Path))
		if p == path {
			return fmt.Errorf("path is already used by %s", n)
		}
		if strings.HasPrefix(path, p+string(os.PathSeparator)) || strings.HasPrefix(p, path+string(os.PathSeparator)) {
			return fmt.Errorf("endpoint paths may not be nested")
		}
	}
	info, err := os.Stat(path)
	if err != nil {
		return fmt.Errorf("path must exist: %w", err)
	}
	if !info.IsDir() {
		return fmt.Errorf("path is not a directory")
	}
	f, err := os.CreateTemp(path, ".skillfleet-write-test-")
	if err != nil {
		return fmt.Errorf("path is not writable: %w", err)
	}
	f.Close()
	os.Remove(f.Name())
	return nil
}

func DefaultConfigPath() string {
	if p := os.Getenv("SKILLFLEET_CONFIG"); p != "" {
		return p
	}
	h, err := os.UserHomeDir()
	if err != nil {
		return ".config/skillfleet/skillfleet.toml"
	}
	return filepath.Join(h, ".config", "skillfleet", "skillfleet.toml")
}
func expand(path string) string {
	if path == "~" {
		h, _ := os.UserHomeDir()
		return h
	}
	if strings.HasPrefix(path, "~/") {
		h, _ := os.UserHomeDir()
		return filepath.Join(h, path[2:])
	}
	return path
}
func sourcePath(c Config, s Skill, endpoint string) string {
	p := s.Source
	if v, ok := s.SourceOverrides[endpoint]; ok {
		p = v
	}
	p = expand(p)
	if filepath.IsAbs(p) {
		return filepath.Clean(p)
	}
	return filepath.Join(expand(c.Library), p)
}
func LoadSnapshot(path string) (Snapshot, error) {
	var c Config
	if _, err := toml.DecodeFile(path, &c); err != nil {
		return Snapshot{}, fmt.Errorf("read config %s: %w", path, err)
	}
	if c.Schema != 1 {
		return Snapshot{}, fmt.Errorf("unsupported schema %d, expected 1", c.Schema)
	}
	if c.Endpoints == nil {
		c.Endpoints = map[string]Endpoint{}
	}
	if c.Skills == nil {
		c.Skills = map[string]Skill{}
	}
	s := Snapshot{ConfigPath: path, Config: c}
	for name, skill := range c.Skills {
		for _, target := range skill.Targets {
			src := sourcePath(c, skill, target)
			ep, ok := c.Endpoints[target]
			r := Route{Skill: name, Endpoint: target, Source: src, State: RouteUnknown}
			if ok {
				r.Destination = filepath.Join(expand(ep.Path), name)
				r.State = inspectRoute(src, r.Destination)
			}
			s.Routes = append(s.Routes, r)
		}
	}
	sort.Slice(s.Routes, func(i, j int) bool {
		if s.Routes[i].Skill == s.Routes[j].Skill {
			return s.Routes[i].Endpoint < s.Routes[j].Endpoint
		}
		return s.Routes[i].Skill < s.Routes[j].Skill
	})
	return s, nil
}
func inspectRoute(source, destination string) RouteState {
	info, err := os.Lstat(destination)
	if os.IsNotExist(err) {
		return RouteMissing
	}
	if err != nil {
		return RouteBroken
	}
	if info.Mode()&os.ModeSymlink == 0 {
		return RouteConflict
	}
	actual, aerr := filepath.EvalSymlinks(destination)
	expected, eerr := filepath.EvalSymlinks(source)
	if aerr != nil || eerr != nil {
		return RouteBroken
	}
	if filepath.Clean(actual) == filepath.Clean(expected) {
		return RouteOK
	}
	return RouteBroken
}
func (s Snapshot) SkillNames() []string {
	names := make([]string, 0, len(s.Config.Skills))
	for n := range s.Config.Skills {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
}
func (s Snapshot) EndpointNames() []string {
	names := make([]string, 0, len(s.Config.Endpoints))
	for n := range s.Config.Endpoints {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
}
func (s Snapshot) StateCounts() map[RouteState]int {
	out := map[RouteState]int{}
	for _, r := range s.Routes {
		out[r.State]++
	}
	return out
}
