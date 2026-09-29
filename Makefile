# eml-linux — build, test, run
CARGO ?= cargo
BIN   := target/release
ROOT  ?= /tmp/eml-use

.PHONY: all build release test demo tui up init status verify clean

all: release

build:
	$(CARGO) build

release:
	$(CARGO) build --release

test:
	$(CARGO) test

# scaffold a working root, bring the stack up for 3s, then show status
demo: release
	$(BIN)/emlctl init --root $(ROOT)
	$(BIN)/emlctl up   --root $(ROOT) --max-runtime 3 || true
	$(BIN)/emlctl status --root $(ROOT)

init: release
	$(BIN)/emlctl init --root $(ROOT)

up: release
	$(BIN)/emlctl up --root $(ROOT)

tui: release
	$(BIN)/emlctl tui --root $(ROOT)

status: release
	$(BIN)/emlctl status --root $(ROOT)

verify: release
	$(BIN)/emlctl verify --root $(ROOT)

clean:
	$(CARGO) clean
