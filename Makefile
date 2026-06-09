.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	install -m 755 target/release/cce-server ~/.local/bin/cce-server

run:
	cargo run

clean:
	cargo clean
