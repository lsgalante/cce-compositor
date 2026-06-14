.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	install -m 755 target/release/cce-client ~/.local/bin/cce-client
	install -m 755 target/release/clearctl ~/.local/bin/clearctl
	install -m 755 target/release/clear-inspector ~/.local/bin/clear-inspector

run:
	cargo run

clean:
	cargo clean
