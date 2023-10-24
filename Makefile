prefix=/usr/local
exec_prefix=$(prefix)
bindir=$(exec_prefix)/bin
exec=inf

.phony: all
all:

.phony: install
install: inf
	install -m755 inf $(bindir)/$(exec)

.phony: check
check: shellcheck
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'
	@printf '\033[0;32mChecks succeded!\033[0m\n'
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'

.phony: shellcheck
shellcheck: inf
	shellcheck -s bash -S style $<
