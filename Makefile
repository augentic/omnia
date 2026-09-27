# Convenience pass-through to mise. Installs mise on first run if missing.
MISE := $(shell command -v mise 2>/dev/null)
ifeq ($(MISE),)
MISE := $(HOME)/.local/bin/mise
endif

.PHONY: %
%:
	@if [ "$@" = "$(firstword $(MAKECMDGOALS))" ]; then \
		if [ ! -x "$(MISE)" ]; then \
			echo "mise not found; installing to $(HOME)/.local/bin (https://mise.run)"; \
			curl -fsSL https://mise.run | sh; \
		fi; \
		"$(MISE)" run "$@" -- $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS)); \
	fi
