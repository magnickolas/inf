prefix=/usr/local
exec_prefix=$(prefix)
bindir=$(exec_prefix)/bin

datarootdir=$(prefix)/share
mandir=$(datarootdir)/man
man1dir=$(mandir)/man1

exec=inf
man1_pages=$(exec).1
target_dir=$(shell cargo metadata --format-version=1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
release_bin=$(target_dir)/release/$(exec)

all: build

build:
	cargo build

release:
	cargo build --release

install: install-bin install-man

install-bin: release
	@mkdir -p $(DESTDIR)$(bindir)/
	install -m755 $(release_bin) $(DESTDIR)$(bindir)/$(exec)

install-man: $(man1_pages)
	@mkdir -p $(DESTDIR)$(man1dir)/
	install -m644 $^ $(DESTDIR)$(man1dir)/

uninstall: uninstall-bin uninstall-man

uninstall-bin:
	rm -f $(DESTDIR)$(bindir)/$(exec)

uninstall-man:
	rm -f $(patsubst %,$(DESTDIR)$(man1dir)/%,$(man1_pages))

check: lint test
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'
	@printf '\033[0;32mChecks succeded!\033[0m\n'
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'

lint:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

test: build
	cargo test
