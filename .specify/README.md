# Specify Tools

This directory serves two purposes, shared via git submodules:

1. **Cursor plugin** — rules, skills, and references that power Omnia code
  generation in Cursor. Shared to the plugins repository.
2. **Specify schema** — workflow definition, configuration, and templates for
  the spec-driven development framework. Shared to repositories that use the
   Specify workflow.

These concerns are co-located because the schema's `apply` phase directly
invokes the plugin's skills, making them a single versioned unit.

## Directory layout

```
.cursor-plugin/    Plugin manifest (plugin.json)
rules/             Cursor rules (.mdc files)
skills/            Plugin skills (guest-writer, crate-writer, test-writer, code-reviewer)
references/        Reference documentation consumed by skills
schemas/omnia/     Specify schema — workflow definition, config, and templates
mcp.json           MCP server configuration
```

