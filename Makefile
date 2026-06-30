.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	@if [ -f ../target/release/cce-fx ]; then \
		install -m 755 ../target/release/cce-fx ~/.local/bin/cce-fx; \
		ln -sf cce-fx ~/.local/bin/cce; \
	else \
		echo "Error: cce-fx binary not found"; exit 1; \
	fi
	@if [ -f ../target/release/cce-wallpaper ]; then \
		install -m 755 ../target/release/cce-wallpaper ~/.local/bin/cce-wallpaper; \
	fi
	install -m 755 scripts/cce-desktop-menu ~/.local/bin/cce-desktop-menu
	install -m 755 scripts/cce-app-menu ~/.local/bin/cce-app-menu

run:
	cargo run --bin cce-fx

clean:
	cargo clean
