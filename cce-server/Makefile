.PHONY: build install run clean

build:
	cargo build --release

install: build
	mkdir -p ~/.local/bin
	@if [ -f target/release/cce-server ]; then \
		install -m 755 target/release/cce-server ~/.local/bin/cce-server; \
	elif [ -f ../target/release/cce-server ]; then \
		install -m 755 ../target/release/cce-server ~/.local/bin/cce-server; \
	elif [ -f ../../target/release/cce-server ]; then \
		install -m 755 ../../target/release/cce-server ~/.local/bin/cce-server; \
	else \
		echo "Error: cce-server binary not found"; exit 1; \
	fi

run:
	cargo run

clean:
	cargo clean
