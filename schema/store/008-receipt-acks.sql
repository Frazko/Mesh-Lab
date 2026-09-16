-- A recipient keeps its signed delivery receipt as evidence, but stops
-- scheduling it once the certified origin returns a matching signed ACK.
CREATE TABLE receipt_acknowledgements (
  object_id BLOB PRIMARY KEY REFERENCES local_deliveries(object_id),
  receipt_id BLOB NOT NULL CHECK(length(receipt_id)=32),
  acknowledged_at INTEGER NOT NULL CHECK(acknowledged_at>=0)
);
CREATE INDEX receipt_acknowledgements_receipt ON receipt_acknowledgements(receipt_id);
PRAGMA user_version=8;
