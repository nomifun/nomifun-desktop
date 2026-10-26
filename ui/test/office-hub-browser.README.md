# Feishu document management browser acceptance

Run `bun run --cwd ui dev --port 5191 --host 127.0.0.1` from the repository root and open
`http://127.0.0.1:5191/test/office-hub-browser.html` in a desktop browser.
The page imports the actual Office Hub renderer. All business methods and the
browser opener are simulated; it never accesses a backend, credentials, remote
documents or external websites. The footer records writes and browser links.

At 880×600 and a wider desktop viewport:

1. Click a document name or 浏览器打开. The footer records its exact URL;
   the document list remains visible without a content preview or editor.
2. Search for a title and clear the search to return to the list.
3. Create a document by title. Only the title is requested; it appears in the list.
4. Open 删除, check the title and URL, then cancel. No write is recorded.
5. Confirm deletion. The item disappears and exactly one write is recorded.
6. Open `?scenario=pending`, confirm a deletion, close the dialog, and verify
   the row disables deletion and points to the original operation.
7. Open `?scenario=empty` and verify the create and open-link entries remain usable.

Browser results use simulated business data. Rust integration tests separately
cover permissions, CLI commands, committed receipts and pending fences.
Neither is evidence of a live Feishu tenant operation.
