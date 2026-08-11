# Crawler

A crawl job walks a website from one or more seed URLs, extracts the readable
content of each page, and files the result into a knowledge base. You configure
it from the **Crawler** page in the sidebar.

> Need a single page rather than a whole site? A knowledge base can take a URL
> source directly — see [MCP & Skills](./mcp-and-skills.md). The crawler is for
> following links across many pages.

## What a job does

`nomifun-crawl` is a durable URL frontier plus a worker pool:

- The **frontier** lives in SQLite (`crawl_jobs` / `crawl_tasks`). Every URL is
  normalized and fingerprinted, so the same page is never queued twice within a
  job.
- **Workers** claim tasks atomically, hold a renewable lease, and submit results
  under a fencing token. A worker that crashes, hangs, or loses the network
  simply stops renewing; its task returns to the queue automatically. There is
  no liveness probe to configure and nothing to restart by hand.
- Discovered links are scope-checked, depth-limited, and counted against the
  job's URL budget before they are enqueued.

Because the queue is durable, closing the app mid-crawl loses at most the pages
that were in flight. Start the job again and it resumes where it stopped.

## Creating a job

| Field | What it does |
| --- | --- |
| **Seed URLs** | One per line. The crawl starts here, and by default cannot leave these sites. |
| **Max depth** | How many link hops away from a seed the crawler may go. `0` means seeds only. |
| **Max URLs** | Hard ceiling on how many URLs the job will ever queue. |
| **Concurrency** | How many pages the job fetches at once, across all hosts. |
| **Per-host concurrency** | How many at once against any single host. This is enforced inside the queue allocator, so it cannot be exceeded even with many workers. |
| **Delay between requests** | Minimum gap between two requests to the same host. |
| **Render mode** | `Auto` and `HTTP only` both fetch over plain HTTP today. Browser rendering is not available yet. |
| **Stay on the seed's site** | Restricts the crawl to the seeds' registrable domains (`docs.example.com` and `example.com` count as the same site). |
| **Respect robots.txt** | On by default. See the compliance section below. |
| **Knowledge base ID** | Where pages are written. Leave empty to crawl without saving content — useful for a dry run. |
| **Stage into the review inbox** | On by default: pages land in the knowledge base's review inbox with a diff preview instead of being written directly. |

## Politeness

The crawler is built to be a well-behaved client, and these are not optional
extras — they are what makes a crawl survivable over time.

- **robots.txt** is fetched once per host and cached for 30 minutes. A `404`
  means the file is absent and the whole site is fair game; a `5xx` means the
  file is undefined and the crawler treats the site as fully disallowed, per
  RFC 9309.
- A site's own **`Crawl-delay`** is honoured whenever it is stricter than your
  configured delay.
- **`Retry-After`** on a `429` or `503` is obeyed directly (delta-seconds form).
  Without it, the crawler backs off exponentially from 1s up to 5 minutes.
- After five consecutive failures a host is **cut off** for 5 minutes. Other
  hosts in the same job keep running.
- `<meta name="robots" content="noindex">` skips ingestion of that page;
  `nofollow` (meta or per-link `rel`) stops its links from being followed.
- Requests carry an identifiable agent string, `NomiFun-Crawler/1.0`.

## What gets saved

Each page is run through a readability extractor to strip navigation, ads, and
footers, then converted to Markdown. Pages are written to
`crawl/{job-name}/{page-slug}.md` with front matter recording the source URL.

The content hash is computed over the **extracted Markdown**, not the raw HTML,
so rotating ads and CSRF tokens do not make an unchanged page look modified. On
a re-crawl, `ETag` / `Last-Modified` are sent and an unchanged page is skipped
without a rewrite — its links are still followed.

Only HTML is ingested in this release. PDFs, images, and JSON responses are
recorded as skipped.

## Reading the progress

The job list shows a live progress bar backed by WebSocket events
(`crawl.progress`, `crawl.task`, `crawl.finished`). **URLs** opens the per-task
list with each page's status, HTTP code, and error.

A job ends as:

- **Done** — the queue drained.
- **Failed** — every task failed and none succeeded.
- **Cancelled** — you stopped it. Tasks that were in flight are not settled;
  their leases lapse within a minute and they return to `pending`, so restarting
  the job picks them up again.

**Retry failed** requeues every parked task and clears its attempt budget. Use
it after fixing a scope rule or restoring access — a task that failed three
times is otherwise left alone on purpose.

## Limits in this release

- No browser rendering. A job set to browser mode fails rather than silently
  fetching the unrendered HTML shell.
- Single-machine only. The claim protocol is already multi-node safe, but the
  remote worker path is not wired up.
- No scheduled re-crawls yet.
- Worker pools are per job, so running several jobs at once adds up their
  concurrency settings with no global cap.

## Where it lives

- Backend: [`crates/backend/nomifun-crawl/`](../../crates/backend/nomifun-crawl/)
- Schema: [`024_crawl_jobs_and_tasks.sql`](../../crates/backend/nomifun-db/migrations/024_crawl_jobs_and_tasks.sql)
- Frontend: [`ui/src/renderer/pages/crawl/`](../../ui/src/renderer/pages/crawl/)
- Design notes: [`docs/specs/2026-08-05-distributed-crawler-design.zh.md`](../specs/2026-08-05-distributed-crawler-design.zh.md)
