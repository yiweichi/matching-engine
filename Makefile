CARGO      = cargo
BIN        = target/release/matching-engine
RUSTFLAGS  = -C target-cpu=native

export RUSTFLAGS

.PHONY: build test clippy fmt bench bench-pin clean

build:
	$(CARGO) build --release

test:
	$(CARGO) test

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt

bench: build
	$(BIN)

bench-pin: build
	taskset -c $(or $(word 2,$(MAKECMDGOALS)),0) $(BIN)

%:
	@:

clean:
	$(CARGO) clean
