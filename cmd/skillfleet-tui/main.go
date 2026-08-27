package main

import (
	"flag"
	"fmt"
	"os"

	"github.com/PhoenixSmith/skillfleet/tui/internal/tui"
	tea "github.com/charmbracelet/bubbletea"
)

func main() {
	config := flag.String("config", tui.DefaultConfigPath(), "path to skillfleet.toml")
	flag.Parse()
	snapshot, err := tui.LoadSnapshot(*config)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	p := tea.NewProgram(tui.NewModel(snapshot), tea.WithAltScreen())
	if _, err := p.Run(); err != nil {
		fmt.Fprintln(os.Stderr, "run TUI:", err)
		os.Exit(1)
	}
}
