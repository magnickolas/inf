PREFIX:="$(HOME)/.local/bin"
EXEC:="inf"

ENTR_DIR:="./third-party/entr"

.phony: all
all:

.phony: install
install: inf
	cp $< $(PREFIX)/$(EXEC)
	chmod +x $(PREFIX)/$(EXEC)

.phony: check
check: shellcheck
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'
	@printf '\033[0;32mChecks succeded!\033[0m\n'
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'

.phony: shellcheck
shellcheck: inf
	shellcheck -s bash -S style $<
