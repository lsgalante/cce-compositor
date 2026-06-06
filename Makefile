.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	install -m 755 target/release/clear-computing-environment-server ~/.local/bin/clear-computing-environment-server

run:
	cargo run

clean:
	cargo clean
