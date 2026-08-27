package tui

import (
	"fmt"
	"os"
	"os/exec"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
)

type actionResultMsg struct {
	title, output string
	err           error
	reload        bool
	doctor        bool
}

type Runner interface {
	Run(config string, args ...string) (string, error)
}
type CLIRunner struct{ Binary string }

func (r CLIRunner) Run(config string, args ...string) (string, error) {
	bin := r.Binary
	if bin == "" {
		bin = os.Getenv("SKILLFLEET_CLI")
	}
	if bin == "" {
		bin = "skillfleet"
	}
	b, err := exec.Command(bin, append([]string{"--config", config}, args...)...).CombinedOutput()
	out := strings.TrimSpace(string(b))
	if err != nil {
		return out, fmt.Errorf("%s", err)
	}
	return out, nil
}
func runAction(r Runner, config, title string, args ...string) tea.Cmd {
	return func() tea.Msg {
		out, err := r.Run(config, args...)
		return actionResultMsg{title: title, output: out, err: err}
	}
}

type applyResultMsg struct {
	output string
	err    error
}

func runApply(r Runner, config string, changes []Change, force bool) tea.Cmd {
	return func() tea.Msg {
		var out []string
		step := func(label string, args ...string) *applyResultMsg {
			s, err := r.Run(config, args...)
			if s != "" {
				out = append(out, s)
			}
			if err != nil {
				return &applyResultMsg{strings.Join(out, "\n"), fmt.Errorf("%s: %w", label, err)}
			}
			return nil
		}
		// Endpoint removals run last, behind an interim sync, so staged
		// unroutes detach their symlinks while the endpoint is still managed;
		// removing first would orphan the links forever.
		var removes, others []Change
		for _, c := range changes {
			if c.Kind == ChangeEndpointRemove {
				removes = append(removes, c)
			} else {
				others = append(others, c)
			}
		}
		for _, c := range others {
			var args []string
			switch c.Kind {
			case ChangeRoute:
				args = []string{"skill", "route", c.Name, "--to"}
				args = append(args, c.Targets...)
			case ChangeEndpointAdd, ChangeEndpointEdit:
				args = []string{"endpoint", "ensure", c.Name, c.Path}
				if c.Vacuum != nil {
					if *c.Vacuum {
						args = append(args, "--vacuum")
					} else {
						args = append(args, "--no-vacuum")
					}
				}
			case ChangeSkillAdd:
				args = []string{"skill", "add", c.Name, "--source", c.Path}
			}
			if m := step(c.Label(), args...); m != nil {
				return *m
			}
		}
		syncArgs := []string{"sync"}
		if force {
			syncArgs = append(syncArgs, "--force")
		}
		if len(removes) > 0 && len(others) > 0 {
			if m := step("sync", syncArgs...); m != nil {
				return *m
			}
		}
		for _, c := range removes {
			if m := step(c.Label(), "endpoint", "remove", c.Name); m != nil {
				return *m
			}
		}
		if m := step("sync", syncArgs...); m != nil {
			return *m
		}
		s, err := r.Run(config, "doctor")
		if s != "" {
			out = append(out, "Doctor:\n"+s)
		}
		if err != nil {
			return applyResultMsg{strings.Join(out, "\n"), fmt.Errorf("doctor: %w", err)}
		}
		return applyResultMsg{output: strings.Join(out, "\n")}
	}
}
