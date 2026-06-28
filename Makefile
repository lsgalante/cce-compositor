.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	@if [ -f ../target/release/cce ]; then \
		install -m 755 ../target/release/cce ~/.local/bin/cce; \
	else \
		echo "Error: cce binary not found"; exit 1; \
	fi
	install -m 755 scripts/cce-desktop-menu ~/.local/bin/cce-desktop-menu
	install -m 755 scripts/cce-app-menu ~/.local/bin/cce-app-menu

run:
	cargo run --bin cce

clean:
	cargo clean
