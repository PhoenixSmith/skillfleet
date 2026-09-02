VERSION  := 0.5.0
PREFIX  ?= $(HOME)/.local
BINDIR  ?= $(PREFIX)/bin
MANDIR  ?= $(PREFIX)/share/man/man1

.PHONY: build test install uninstall man completions clean

build:
	cargo build --release
	go build -o target/release/skillfleet-tui ./cmd/skillfleet-tui

test:
	cargo test --release
	go test ./...

install: build
	install -d $(BINDIR)
	install -m 0755 target/release/skillfleet target/release/skillfleet-tui $(BINDIR)/

uninstall:
	rm -f $(BINDIR)/skillfleet $(BINDIR)/skillfleet-tui

man: build
	install -d $(MANDIR)
	target/release/skillfleet man > $(MANDIR)/skillfleet.1

completions: build
	target/release/skillfleet completions bash > target/release/skillfleet.bash
	target/release/skillfleet completions zsh > target/release/skillfleet.zsh
	target/release/skillfleet completions fish > target/release/skillfleet.fish
	@echo "completion scripts written to target/release/skillfleet.{bash,zsh,fish}"
	@echo "install e.g.: install -m 0644 target/release/skillfleet.bash /etc/bash_completion.d/"

clean:
	cargo clean
	rm -f target/release/skillfleet-tui target/release/skillfleet.{bash,zsh,fish}