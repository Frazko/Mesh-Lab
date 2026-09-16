-- A signed receipt ACK may be the only signal that stops a remote phone from
-- retrying. Preserve it across a relay restart just as we preserve receipts.
CREATE TABLE relay_receipt_ack_outbox (
  ack_id BLOB PRIMARY KEY CHECK(length(ack_id)=32),
  ack BLOB NOT NULL CHECK(length(ack)>0 AND length(ack)<=1024),
  relay_id BLOB NOT NULL CHECK(length(relay_id)=16),
  origin BLOB NOT NULL CHECK(length(origin)=32),
  previous_hop BLOB NOT NULL CHECK(length(previous_hop)=32),
  received_from BLOB NOT NULL CHECK(length(received_from)=32),
  hops INTEGER NOT NULL CHECK(hops>=0 AND hops<=16),
  hop_limit INTEGER NOT NULL CHECK(hop_limit>0 AND hop_limit<=16),
  expires_at INTEGER NOT NULL CHECK(expires_at>0),
  custodied_at INTEGER NOT NULL CHECK(custodied_at>=0)
);
CREATE INDEX relay_receipt_ack_expiry ON relay_receipt_ack_outbox(expires_at);
PRAGMA user_version=9;
