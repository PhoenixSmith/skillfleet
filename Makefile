VERSION  := 0.2.0
PREFIX  ?= $(HOME)/.local
BINDIR  ?= $(PREFIX)/bin

.PHONY: build test install clean

build:
	cargo build --release
	go build -o target/release/skillfleet-tui ./cmd/skillfleet-tui

test:
	cargo test --release
	go test ./...

install: build
	install -d $(BINDIR)
	install -m 0755 target/release/skillfleet target/release/skillfleet-tui $(BINDIR)/

clean:
	cargo clean
	rm -f target/release/skillfleet-tui