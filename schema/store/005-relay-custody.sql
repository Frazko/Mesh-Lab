-- A completed object received from another member remains eligible for
-- store-and-forward after process death. This is separate from `outbox`: only
-- an origin's outbox is retired by recipient delivery receipts.
CREATE TABLE relay_outbox (
  object_id BLOB PRIMARY KEY REFERENCES objects(id),
  custodied_at INTEGER NOT NULL CHECK(custodied_at>=0)
);
-- Metadata is the canonical per-hop relay frame supplied by the authenticated
-- runtime after it passes the dedupe/hop gate. The protected object bytes are
-- still stored in `objects`/`chunks`; this table never contains plaintext.
CREATE TABLE relay_metadata (
  object_id BLOB PRIMARY KEY REFERENCES relay_outbox(object_id),
  relay_id BLOB NOT NULL CHECK(length(relay_id)=16),
  origin BLOB NOT NULL CHECK(length(origin)=32),
  previous_hop BLOB NOT NULL CHECK(length(previous_hop)=32),
  hops INTEGER NOT NULL CHECK(hops>=0 AND hops<=16),
  hop_limit INTEGER NOT NULL CHECK(hop_limit>0 AND hop_limit<=16),
  expires_at INTEGER NOT NULL CHECK(expires_at>0)
);
PRAGMA user_version=5;
