# dynamically target Makefile.toml
# .PHONY: %
# %:
# 	@cargo make $@

.PHONY: %
%:
	@if [ "$@" = "$(firstword $(MAKECMDGOALS))" ]; then \
		cargo make "$@" $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS)); \
	fi