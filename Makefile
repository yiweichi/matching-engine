CARGO      = cargo
BIN        = target/release/matching-engine
RUSTFLAGS  = -C target-cpu=native
RUN        = $(if $(CPU),taskset -c $(CPU) ,)$(BIN)

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
	$(RUN) bench

bench-scenario: build
	@test -n "$(SCENARIO)" || (echo "usage: make bench-scenario SCENARIO=<name> [DEPTH=n] [LEVELS=n] [ORDERS=n] [CPU=n]"; exit 1)
	$(RUN) bench --scenario $(SCENARIO) $(if $(DEPTH),--depth $(DEPTH),) $(if $(LEVELS),--levels $(LEVELS),) $(if $(ORDERS),--orders $(ORDERS),)

profile-scenario: build
	@test -n "$(SCENARIO)" || (echo "usage: make profile-scenario SCENARIO=<name> [DEPTH=n] [LEVELS=n] [ORDERS=n] [REPEAT=n] [CPU=n]"; exit 1)
	$(RUN) profile --scenario $(SCENARIO) $(if $(DEPTH),--depth $(DEPTH),) $(if $(LEVELS),--levels $(LEVELS),) $(if $(ORDERS),--orders $(ORDERS),) $(if $(REPEAT),--repeat $(REPEAT),)

%:
	@:

clean:
	$(CARGO) clean
