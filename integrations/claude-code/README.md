# Claude Code plugin

Gives Claude the Anthill tools (via MCP) and a skill describing the find / understand / fix / replay loop.

Local (stdio, runs in your project directory, no server needed):

    claude mcp add anthill -- anthill mcp

Remote server (HTTP with a bearer token):

    claude mcp add --transport http anthill https://anthill.example.com/mcp \
      --header "Authorization: Bearer $ANTHILL_TOKEN"

Use a `read` token for analysis-only access; `start_run` needs a `run` token.

To install as a plugin, point Claude Code at this directory (it contains `.claude-plugin/plugin.json`,
`.mcp.json`, a skill and a `/anthill-test` command).
