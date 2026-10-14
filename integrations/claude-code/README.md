# Claude Code plugin

Gives Claude the Hairspring tools (via MCP) and a skill describing the find / understand / fix / replay loop.

Local (stdio, runs in your project directory, no server needed):

    claude mcp add hairspring -- hairspring mcp

Remote server (HTTP with a bearer token):

    claude mcp add --transport http hairspring https://hairspring.example.com/mcp \
      --header "Authorization: Bearer $HAIRSPRING_TOKEN"

Use a `read` token for analysis-only access; `start_run` needs a `run` token.

To install as a plugin, point Claude Code at this directory (it contains `.claude-plugin/plugin.json`,
`.mcp.json`, a skill and a `/hairspring-test` command).
