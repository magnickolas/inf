prefix=/usr/local
exec_prefix=$(prefix)
bindir=$(exec_prefix)/bin

datarootdir=$(prefix)/share
mandir=$(datarootdir)/man
man1dir=$(mandir)/man1

exec=inf
man1_pages=$(exec).1

.PHONY: all
all:

.PHONY: install install-bin install-man
install: install-bin install-man

install-bin: inf
	@mkdir -p $(DESTDIR)$(bindir)/
	install -m755 inf $(DESTDIR)$(bindir)/$(exec)

install-man: $(man1_pages)
	@mkdir -p $(DESTDIR)$(man1dir)/
	install -m644 $^ $(DESTDIR)$(man1dir)/

.PHONY: uninstall uninstall-bin uninstall-man
uninstall: uninstall-bin uninstall-man

uninstall-bin:
	rm -f $(DESTDIR)$(bindir)/$(exec)

uninstall-man:
	rm -f $(patsubst %,$(DESTDIR)$(man1dir)/%,$(man1_pages))

.PHONY: check test shellcheck
check: shellcheck test
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'
	@printf '\033[0;32mChecks succeded!\033[0m\n'
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'

shellcheck: inf
	shellcheck -s bash -S style $<

test:
	bats tests
