# Feishu connection and document browser acceptance

Run `bun run --cwd ui dev --port 5191 --host 127.0.0.1` from the repository root and open
`http://127.0.0.1:5191/test/office-hub-browser.html` in a desktop browser.
The page imports the actual Office Hub renderer. All business methods and the
browser opener are simulated; it never accesses a backend, credentials, remote
documents or external websites. The footer records simulated actions, writes and
the exact browser links. It also provides scenario selection and explicit auth
completion controls; those controls belong to the QA harness, not the product.

At 880×600 and a wider desktop viewport, the top navigation contains only
文档、日程与任务, and the default page is 文档. Base tables use the 多维表格 filter
under 文档.

## Connection states

Use the footer selector or append one of these query strings:

| Scenario | Initial state |
| --- | --- |
| `?scenario=missing` | No CLI component; no account or selected application |
| `?scenario=needs-login` | Ready component and saved QA application; no signed-in user |
| `?scenario=connected` | Ready component, connected QA user and granted business scopes |
| `?scenario=expired` | Saved QA application with an expired user login |
| `?scenario=broken` | Existing component cannot start; it must not be presented as missing |
| `?scenario=unconfigured` | Ready component, with no selected application |

Check the first-use and account-change flows:

1. In `missing`, inspect the initial page, then prepare the component. The mock
   reports preparing before becoming ready and discovering the saved QA profile.
   This never downloads, installs or runs anything.
2. In `needs-login`, start login. Follow the visible authorization prompt and use
   the footer's **模拟完成授权** button. The renderer observes completion through
   its normal auth polling. Browser-open actions only record a link.
3. Identity-only login grants `offline_access` without business permissions.
   Grant document or agenda reading from the corresponding product control, then
   complete that distinct request with the same footer button. Configuration
   flows complete application setup first and leave user login as a separate step.
4. In `connected`, open account settings and use **退出登录** or **更换账号**.
   Confirm the chosen action; the mock clears only its in-memory current actor.
   Start a new login and choose **模拟另一账号登录** to finish with a different QA
   user. Verify the displayed account changes and the old input cannot be reused.
5. Start a login or permission request and choose **模拟拒绝授权**, or cancel it
   through the product. Verify retry remains possible and no write occurs.
6. Inspect `expired` and `broken` independently. The first offers a user-login
   recovery; the second keeps component repair and account login distinct.
7. Component path selection, automatic discovery, read-only version checks,
   connection enabling and disabling are mocked locally. They never change a real
   CLI, account, credential or installed component.

## Document behavior

The existing `list`, `paged`, `error`, `empty` and `pending` scenarios remain
available with a connected account. Check the following:

1. Click a document name or 浏览器打开. The footer records its exact URL;
   the document list remains visible without a content preview or editor.
2. Search for a title and clear the search to return to the list. Filters apply
   to the simulated server collection, before pagination.
3. Create a document by title. Only the title is requested; it appears in the list.
4. Open 删除, check the title and URL, then cancel. No write is recorded.
5. Confirm deletion. The item disappears and exactly one write is recorded.
6. Open `?scenario=pending`, confirm a deletion, close the dialog, and verify
   the row disables deletion and points to the original operation.
7. Open `?scenario=paged`; without a keyword, page through 25 existing documents.
8. Open `?scenario=error`; verify a read error and retry entry without an empty
   account claim.
9. Open `?scenario=empty` and verify the create and open-link entries remain usable.

Browser results use simulated business data. Rust integration tests separately
cover permissions, CLI commands, committed receipts and pending fences.
Neither is evidence of a live Feishu tenant operation. Changing the QA scenario
reloads the fixture and resets its temporary state.
