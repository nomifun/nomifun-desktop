-- Per-tag notification event filter. Comma-separated subset of done/failed/needs_review.
-- Default keeps current behavior (all three fire).
ALTER TABLE tag_settings ADD COLUMN notify_events TEXT NOT NULL DEFAULT 'done,failed,needs_review';
