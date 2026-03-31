# Library, memory, documents, and MCP

These are related, but they are not the same surface.

Paths:

- `Library > Documents`
- `Settings > Knowledge > Memory`
- `Settings > Knowledge > Memory > Facts`
- `Settings > Knowledge > Memory > Preferences`
- `Settings > Knowledge > Memory > User Data`
- `Settings > Knowledge > Memory > Knowledge`
- `Settings > Knowledge > MCP Servers`

How to think about them:

- `Library > Documents`: uploaded files and indexed document context.
- `Facts`: durable facts the system has stored.
- `Preferences`: long-lived user preferences and rules.
- `User Data`: captured notes, links, and user-supplied structured data.
- `Knowledge`: reusable knowledge-base items, including bundled product-help docs after sync.
- `MCP Servers`: external tool servers that extend what AgentArk can access.

When to use each:

1. Use `Library > Documents` when you want to upload and search files.
2. Use `Settings > Knowledge > Memory > Knowledge` for reusable KB entries, notes, or curated instructions.
3. Use `Facts`, `Preferences`, and `User Data` when the question is about what AgentArk remembers.
4. Use `Settings > Knowledge > MCP Servers` when you want to add or manage external MCP-backed tools.

Verification:

- Uploaded files should appear in `Library > Documents`.
- Reusable knowledge items should appear in `Settings > Knowledge > Memory > Knowledge`.
- If an MCP server is enabled correctly, it should appear in the MCP list and expose its tools/resources.

Common confusion:

- `Documents` are file-centric; `Knowledge` is reusable KB content.
- `Memory` is the structured store; `Knowledge` is only one tab inside that area.
- MCP is external capability extension, not the same thing as the local knowledge base.
