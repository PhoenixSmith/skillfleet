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
		for _, c := range changes {
			var args []string
			switch c.Kind {
			case ChangeRoute:
				args = []string{"skill", "route", c.Name, "--to"}
				args = append(args, c.Targets...)
			case ChangeEndpointAdd, ChangeEndpointEdit:
				args = []string{"endpoint", "ensure", c.Name, c.Path}
			case ChangeEndpointRemove:
				args = []string{"endpoint", "remove", c.Name}
			case ChangeSkillAdd:
				args = []string{"skill", "add", c.Name, "--source", c.Path}
			}
			s, err := r.Run(config, args...)
			if s != "" {
				out = append(out, s)
			}
			if err != nil {
				return applyResultMsg{strings.Join(out, "\n"), fmt.Errorf("%s: %w", c.Label(), err)}
			}
		}
		args := []string{"sync"}
		if force {
			args = append(args, "--force")
		}
		s, err := r.Run(config, args...)
		if s != "" {
			out = append(out, s)
		}
		if err != nil {
			return applyResultMsg{strings.Join(out, "\n"), fmt.Errorf("sync: %w", err)}
		}
		s, err = r.Run(config, "doctor")
		if s != "" {
			out = append(out, "Doctor:\n"+s)
		}
		if err != nil {
			return applyResultMsg{strings.Join(out, "\n"), fmt.Errorf("doctor: %w", err)}
		}
		return applyResultMsg{output: strings.Join(out, "\n")}
	}
}
