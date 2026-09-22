-- Keep one immutable ACK per committed receipt. Re-signing on every drain
-- changed its route identity and defeated radio/relay duplicate suppression.
CREATE TABLE origin_receipt_acks (
  object_id BLOB NOT NULL,
  actor BLOB NOT NULL,
  ack BLOB NOT NULL,
  PRIMARY KEY (object_id, actor),
  FOREIGN KEY (object_id, actor) REFERENCES target_receipts(object_id, actor)
    ON DELETE CASCADE
);
PRAGMA user_version = 11;
