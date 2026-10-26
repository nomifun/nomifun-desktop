# Feishu document editor browser acceptance

Run `bun run --cwd ui dev --port 5191` from the repository root and open
`http://127.0.0.1:5191/test/office-hub-browser.html` in a desktop browser.
The page imports the actual Office Hub renderer. Every business method is
replaced with an explicitly simulated boundary; it never accesses a backend,
Feishu credentials or remote documents.

At 880×600 and a wider desktop viewport:

1. Click 编辑, click the paragraph text, and verify keyboard focus in the input.
2. Save literal Markdown punctuation, a Windows path and a soft line break.
3. Enter 编辑 again; verify the same paragraph remains editable and its raw text
   matches the input. The footer records write count, revision and block ID.
4. Open `?scenario=readonly`; verify the reason and read-only status are visible.
5. Open `?scenario=empty`; insert the first paragraph, save, and edit it again.

This checks real browser interactions with simulated business results. Native
CLI parsing, pagination, preflight and receipts are covered separately by the
`nomifun-office-hub` Rust tests. Neither is evidence of a live Feishu tenant write.
