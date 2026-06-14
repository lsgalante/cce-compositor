.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	@if [ -f target/release/cce-client ]; then \
		install -m 755 target/release/cce-client ~/.local/bin/cce-client; \
		install -m 755 target/release/clearctl ~/.local/bin/clearctl; \
		install -m 755 target/release/clear-inspector ~/.local/bin/clear-inspector; \
	elif [ -f ../target/release/cce-client ]; then \
		install -m 755 ../target/release/cce-client ~/.local/bin/cce-client; \
		install -m 755 ../target/release/clearctl ~/.local/bin/clearctl; \
		install -m 755 ../target/release/clear-inspector ~/.local/bin/clear-inspector; \
	elif [ -f ../../target/release/cce-client ]; then \
		install -m 755 ../../target/release/cce-client ~/.local/bin/cce-client; \
		install -m 755 ../../target/release/clearctl ~/.local/bin/clearctl; \
		install -m 755 ../../target/release/clear-inspector ~/.local/bin/clear-inspector; \
	else \
		echo "Error: cce-client binaries not found"; exit 1; \
	fi

run:
	cargo run

clean:
	cargo clean
