-- A relay may receive a signed delivery receipt while it has no direct route
-- to the origin. Keep the public receipt and already-authenticated routing
-- metadata in SQLCipher so a process restart cannot discard that return hop.
-- This table deliberately has no foreign key to objects: an intermediate relay
-- may forward a receipt for an object it did not retain locally.
CREATE TABLE relay_receipt_outbox (
  receipt_id BLOB PRIMARY KEY CHECK(length(receipt_id)=32),
  receipt BLOB NOT NULL CHECK(length(receipt)>0 AND length(receipt)<=1024),
  relay_id BLOB NOT NULL CHECK(length(relay_id)=16),
  origin BLOB NOT NULL CHECK(length(origin)=32),
  previous_hop BLOB NOT NULL CHECK(length(previous_hop)=32),
  received_from BLOB NOT NULL CHECK(length(received_from)=32),
  hops INTEGER NOT NULL CHECK(hops>=0 AND hops<=16),
  hop_limit INTEGER NOT NULL CHECK(hop_limit>0 AND hop_limit<=16),
  expires_at INTEGER NOT NULL CHECK(expires_at>0),
  custodied_at INTEGER NOT NULL CHECK(custodied_at>=0)
);
CREATE INDEX relay_receipt_expiry ON relay_receipt_outbox(expires_at);
PRAGMA user_version=7;
