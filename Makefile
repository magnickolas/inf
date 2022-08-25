PREFIX:="$(HOME)/.local/bin"
EXEC:="inf"

ENTR_DIR:="./third-party/entr"

all: checks

.phony: install
install: inf
	cp $< $(PREFIX)/$(EXEC)
	chmod +x $(PREFIX)/$(EXEC)

.phony: install_deps
install_deps: entr
	cp $< $(PREFIX)/entr

entr: $(ENTR_DIR)/Makefile
	make -C $(ENTR_DIR)
	cp $(ENTR_DIR)/entr .

$(ENTR_DIR)/Makefile:
	cd $(ENTR_DIR) && ./configure

.phony: checks
checks: shellcheck
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'
	@printf '\033[0;32mChecks succeded!\033[0m\n'
	@printf '\033[0;32m>>>>>>>>>>>>>>>>\033[0m\n'

.phony: shellcheck
shellcheck: inf
	shellcheck -s bash -S style $<
