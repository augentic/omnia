# Convenience pass-through to mise: `make <task> [args]` runs `mise run <task> -- [args]`.
# mise is never installed implicitly; see https://mise.jdx.dev/getting-started.html
MISE := $(shell command -v mise 2>/dev/null)

.PHONY: %
%:
	@if [ "$@" = "$(firstword $(MAKECMDGOALS))" ]; then \
		if [ -z "$(MISE)" ]; then \
			echo "mise not found on PATH; install it first: https://mise.jdx.dev/getting-started.html" >&2; \
			exit 1; \
		fi; \
		"$(MISE)" run "$@" -- $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS)); \
	fi
