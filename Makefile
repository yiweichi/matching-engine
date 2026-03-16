CARGO      = cargo
BIN        = target/release/matching-engine
RUSTFLAGS  = -C target-cpu=native

export RUSTFLAGS

.PHONY: build test clippy fmt pr bench bench-scenario profile-scenario bench-pin clean

build:
	$(CARGO) build --release

test:
	$(CARGO) test

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt

pr: test clippy fmt

bench: build
	$(BIN) bench

bench-scenario: build
	@test -n "$(SCENARIO)" || (echo "usage: make bench-scenario SCENARIO=<name> [DEPTH=n] [LEVELS=n] [ORDERS=n]"; exit 1)
	$(BIN) bench --scenario $(SCENARIO) $(if $(DEPTH),--depth $(DEPTH),) $(if $(LEVELS),--levels $(LEVELS),) $(if $(ORDERS),--orders $(ORDERS),)

profile-scenario: build
	@test -n "$(SCENARIO)" || (echo "usage: make profile-scenario SCENARIO=<name> [DEPTH=n] [LEVELS=n] [ORDERS=n]"; exit 1)
	$(BIN) profile --scenario $(SCENARIO) $(if $(DEPTH),--depth $(DEPTH),) $(if $(LEVELS),--levels $(LEVELS),) $(if $(ORDERS),--orders $(ORDERS),)

bench-pin: build
	taskset -c $(or $(word 2,$(MAKECMDGOALS)),0) $(BIN) bench

%:
	@:

clean:
	$(CARGO) clean
